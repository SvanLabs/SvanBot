//! Reads (`GLOSSARY.md`): one opponent's own departure from the population's action model,
//! learned from their hands alone, and trusted only as far as the evidence and the exploit budget
//! allow.
//!
//! A read counts the opponent's moves per situation. Their frequency at a situation is shrunk
//! toward the population's with a prior strength, then the population model is tilted toward it
//! by the factor `(theirs / population) ^ w`, where `w = budget * n / (n + k)`: the data-biased
//! response's trust in `n` observations, capped by the exploit budget
//! (`docs/research/2026-11/03-opponent-response.md`, option 1). No evidence, or a budget of 0,
//! leaves the population model as it is.

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

/// The action model for one opponent: `population` tilted toward their read, as far as `trust`
/// allows.
pub fn personal(population: &ActionModel, read: &Read, trust: Trust) -> ActionModel {
    population.tilted(|situation, mv| {
        let n = f64::from(read.n(situation));
        let weight = trust.budget.clamp(0.0, 1.0) * n / (n + trust.half);
        let base = population.marginal(situation, mv);
        if weight == 0.0 || base <= 0.0 {
            return 1.0;
        }
        (read.frequency(situation, mv, base, trust.prior) / base).powf(weight)
    })
}

fn situation_index(situation: Situation) -> usize {
    situation.street as usize * 2 + usize::from(situation.facing)
}

fn move_index(mv: Move) -> usize {
    Move::ALL.iter().position(|&m| m == mv).unwrap_or(0)
}
