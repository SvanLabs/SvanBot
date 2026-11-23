//! Reads (`GLOSSARY.md`): one opponent's own departure from the population's action model,
//! learned from their hands alone, and trusted only as far as the evidence and the exploit budget
//! allow.
//!
//! A read counts the opponent's moves per situation. Their frequency at a situation is shrunk
//! toward the population's with a prior strength. At each strength bucket, a share
//! `budget * n / (n + half)` of the moves follows that frequency independently of the cards.
//! The remaining share follows the population's strength-dependent model.

use crate::action::{ActionModel, Move, Seen, Situation};

/// How a read is trusted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trust {
    /// The exploit budget, 0 to 1: how far the model may move toward the read.
    pub budget: f64,
    /// Observations at a situation that earn half the budget.
    pub half: f64,
    /// The prior strength each frequency is shrunk with, in observations.
    pub prior: f64,
}

impl Trust {
    /// Full budget, half trust at 30 observations, shrunk by 20: until the learner fits them.
    pub const DEFAULT: Self = Self {
        budget: 1.0,
        half: 30.0,
        prior: 20.0,
    };
}

/// How an opponent adapts, and so how far their reads are trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Not enough evidence to say how they adapt: no read.
    Unknown,
    /// Plays one strategy: full reads.
    Fixed,
    /// Changed strategy: reads restart from the change.
    Switching,
    /// Learns from how we play: reads are damped.
    Learning,
}

impl Class {
    /// The budget share this class earns.
    pub fn budget(self) -> f64 {
        match self {
            Self::Unknown => 0.0,
            Self::Fixed | Self::Switching => 1.0,
            Self::Learning => 0.25,
        }
    }

    pub const ALL: [Self; 4] = [Self::Unknown, Self::Fixed, Self::Switching, Self::Learning];

    /// The class's name, as a trace or a page writes it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Fixed => "fixed",
            Self::Switching => "switching",
            Self::Learning => "learning",
        }
    }

    /// The class [`Class::name`] gives `text`.
    pub fn named(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.name() == text)
    }
}

/// One opponent's moves, counted per situation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Read {
    counts: [[u32; 6]; Situation::COUNT],
}

impl Read {
    /// Counts one action.
    pub fn observe(&mut self, seen: &Seen) {
        let mv = seen.situation.counted(seen.mv);
        self.counts[situation_index(seen.situation)][move_index(mv)] += 1;
    }

    /// Observations at `situation`.
    pub fn n(&self, situation: Situation) -> u32 {
        self.counts[situation_index(situation)].iter().sum()
    }

    /// Their frequency of `mv` at `situation`, shrunk toward `population` with strength `prior`.
    pub fn frequency(&self, situation: Situation, mv: Move, population: f64, prior: f64) -> f64 {
        let row = &self.counts[situation_index(situation)];
        let n: u32 = row.iter().sum();
        (f64::from(row[move_index(mv)]) + prior * population) / (f64::from(n) + prior)
    }
}

/// Mix the observed frequencies into every strength bucket within the evidence and budget.
pub fn personal(population: &ActionModel, read: &Read, trust: Trust) -> ActionModel {
    population.mixed(
        |situation| {
            let n = f64::from(read.n(situation));
            trust.budget.clamp(0.0, 1.0) * n / (n + trust.half)
        },
        |situation, mv| {
            read.frequency(
                situation,
                mv,
                population.marginal(situation, mv),
                trust.prior,
            )
        },
    )
}

fn situation_index(situation: Situation) -> usize {
    situation.street as usize * 2 + usize::from(situation.facing)
}

fn move_index(mv: Move) -> usize {
    Move::ALL.iter().position(|&m| m == mv).unwrap_or(0)
}
