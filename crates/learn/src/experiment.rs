//! A bot's own experiment (V11-129, the tracer for V11-125): the fixed format it writes from its
//! own record, the choice of which belief to test next, and the reading of the live evidence.
//!
//! The bot does not invent a new simulator or a new gate. It reads the sims it already ran
//! (its own search log), finds the challenger closest to the bar that the sims did *not* put
//! above the champion, and writes the belief the live record can still decide: that this
//! challenger keeps the margin against the reference. The existing guard answers it; a clear
//! promotes, a harm or an undecided run leaves the bot exactly as it was.

use std::collections::BTreeSet;

use svanbot_strategy::params::{Digest, Params};

use crate::paired::Verdict;
use crate::probe;
use crate::search::{Entry, Stage};

/// The fixed format of a self-written experiment.
#[derive(Clone, Debug, PartialEq)]
pub struct Experiment {
    pub claim: Claim,
    pub metric: Metric,
    pub budget: Budget,
    pub stop: StopRule,
}

/// What the bot believes, in the numbers its own search already produced.
#[derive(Clone, Debug, PartialEq)]
pub enum Claim {
    /// The challenger keeps the margin live, whatever its sims against the champion said: the
    /// guard over the reference decides it.
    ChallengerKeepsMargin {
        champion: Digest,
        challenger: Params,
        /// The bot's own sim that chose the challenger, kept beside the belief.
        sim: Verdict,
    },
}

impl Claim {
    /// The challenger the experiment puts forward.
    pub fn challenger(&self) -> Params {
        match self {
            Self::ChallengerKeepsMargin { challenger, .. } => *challenger,
        }
    }

    /// The champion the belief was written about.
    pub fn champion(&self) -> Digest {
        match self {
            Self::ChallengerKeepsMargin { champion, .. } => *champion,
        }
    }
}

/// The measurable outcome the verdict is read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// The live edge over the reference, in bb/100, luck-corrected through the confidence
    /// sequence: the guard's own reading.
    LiveEdge,
}

impl Metric {
    pub const fn name(self) -> &'static str {
        match self {
            Self::LiveEdge => "live_edge",
        }
    }
}

/// How much live play the experiment may take before it expires undecided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub hands: u64,
}

/// When the experiment stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StopRule {
    /// The guard's confidence sequence leaves zero, or the budget is spent undecided.
    Guard { margin_bb: f64 },
}

impl StopRule {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Guard { .. } => "guard",
        }
    }
}

/// The experiment's standing, as its ledger records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A bot has claimed it and its live hands are counting.
    Testing,
    /// The guard cleared at the margin: the bot keeps the challenger.
    Cleared,
    /// The guard found harm: the bot is left as it was.
    Harmful,
    /// The budget was spent without a verdict: the bot is left as it was.
    Undecided,
}

impl Outcome {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Testing => "testing",
            Self::Cleared => "cleared",
            Self::Harmful => "harmful",
            Self::Undecided => "undecided",
        }
    }

    /// Whether the experiment is still running: no outcome yet.
    pub const fn active(self) -> bool {
        matches!(self, Self::Testing)
    }

    /// Whether the bot keeps the challenger from this outcome.
    pub const fn keeps(self) -> bool {
        matches!(self, Self::Cleared)
    }
}

/// What a probe's verdict makes of an experiment.
pub fn outcome(verdict: probe::Verdict) -> Outcome {
    match verdict {
        probe::Verdict::Running => Outcome::Testing,
        probe::Verdict::Cleared | probe::Verdict::Supported => Outcome::Cleared,
        probe::Verdict::Harmful => Outcome::Harmful,
        probe::Verdict::Inconclusive | probe::Verdict::SimGap => Outcome::Undecided,
    }
}

/// Chooses the belief to test next from the bot's own search log: the challenger whose screening
/// or confirmation came closest to the bar (the highest mbb/100 that is not wholly above zero),
/// excluding the champion, the challengers already tried, and anything the search already put
/// over the bar. `None` when the log holds nothing left to ask about.
pub fn choose(
    entries: &[Entry],
    champion: &Params,
    tried: &BTreeSet<Digest>,
    budget: Budget,
    margin_bb: f64,
) -> Option<Experiment> {
    let digest = champion.digest();
    let mut best: Option<(f64, Params, Verdict)> = None;
    for entry in entries {
        let Entry::Simmed { step, verdict } = entry else {
            continue;
        };
        if step.champion != digest
            || step.stage != Stage::First
            || step.challenger == *champion
            || verdict.above_zero()
            || tried.contains(&step.challenger.digest())
        {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(held, ..)| verdict.mbb_per_100 > *held)
        {
            best = Some((verdict.mbb_per_100, step.challenger, *verdict));
        }
    }
    best.map(|(_, challenger, sim)| Experiment {
        claim: Claim::ChallengerKeepsMargin {
            champion: digest,
            challenger,
            sim,
        },
        metric: Metric::LiveEdge,
        budget,
        stop: StopRule::Guard { margin_bb },
    })
}

/// The experiment in words, for the search log and the operator.
pub fn describe(experiment: &Experiment) -> String {
    match &experiment.claim {
        Claim::ChallengerKeepsMargin {
            champion,
            challenger,
            sim,
        } => format!(
            "{} believes {} keeps the margin live (its sim: {:+.0} mbb/100, 95% ±{:.0}, over {} deals), against champion {champion}, over {} hands",
            experiment.metric.name(),
            challenger.digest(),
            sim.mbb_per_100,
            sim.ci95,
            sim.deals,
            experiment.budget.hands
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Step;
    use svanbot_strategy::params::Params;

    fn sim(challenger: Params, mbb: f64) -> Entry {
        Entry::Simmed {
            step: Step {
                champion: Params::DEFAULT.digest(),
                challenger,
                stage: Stage::First,
                seed: 7,
            },
            verdict: Verdict {
                deals: 100,
                mbb_per_100: mbb,
                ci95: 100.0,
            },
        }
    }

    fn budget() -> Budget {
        Budget { hands: 3_000 }
    }

    #[test]
    fn the_closest_challenger_below_the_bar_becomes_the_belief() {
        let below = Params {
            exploit: 0.4,
            ..Params::DEFAULT
        };
        let closer = Params {
            exploit: 0.52,
            ..Params::DEFAULT
        };
        let entries = [
            sim(below, -1_200.0),
            sim(closer, -300.0),
            // Wholly above zero: the search's own bar, and not an experiment's to ask.
            sim(
                Params {
                    exploit: 0.9,
                    ..Params::DEFAULT
                },
                900.0,
            ),
        ];
        let chosen = choose(&entries, &Params::DEFAULT, &BTreeSet::new(), budget(), 20.0).unwrap();
        assert_eq!(chosen.claim.challenger(), closer);
        assert_eq!(chosen.stop, StopRule::Guard { margin_bb: 20.0 });
        assert_eq!(chosen.budget, budget());
    }

    #[test]
    fn a_tried_challenger_is_not_asked_about_again() {
        let challenger = Params {
            exploit: 0.4,
            ..Params::DEFAULT
        };
        let mut tried = BTreeSet::new();
        tried.insert(challenger.digest());
        assert!(
            choose(
                &[sim(challenger, -300.0)],
                &Params::DEFAULT,
                &tried,
                budget(),
                20.0
            )
            .is_none()
        );
    }

    #[test]
    fn a_log_with_nothing_below_the_bar_has_no_belief() {
        let challenger = Params {
            exploit: 0.4,
            ..Params::DEFAULT
        };
        assert!(
            choose(
                &[sim(challenger, 1_000.0)],
                &Params::DEFAULT,
                &BTreeSet::new(),
                budget(),
                20.0
            )
            .is_none()
        );
        assert!(choose(&[], &Params::DEFAULT, &BTreeSet::new(), budget(), 20.0).is_none());
    }

    #[test]
    fn the_guard_verdicts_map_to_outcomes() {
        assert_eq!(outcome(probe::Verdict::Running), Outcome::Testing);
        assert_eq!(outcome(probe::Verdict::Cleared), Outcome::Cleared);
        assert_eq!(outcome(probe::Verdict::Harmful), Outcome::Harmful);
        assert_eq!(outcome(probe::Verdict::Inconclusive), Outcome::Undecided);
        assert_eq!(outcome(probe::Verdict::SimGap), Outcome::Undecided);
        assert!(Outcome::Cleared.keeps());
        assert!(!Outcome::Harmful.keeps());
    }
}
