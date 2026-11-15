//! Fitting the action model on every hand, and the gate a fit must pass before it is used.
//!
//! The opponent's hole cards are the hidden variable. Each hand contributes the chance of the
//! actions seen, summed over every pair the opponent could hold, `Σ_h r₀(h) Π_t π(a_t | h)`, so
//! hands that end in a fold count as much as showdowns. EM fits it: the E-step weighs each pair by
//! that product, and the M-step counts the moves by those weights (`docs/research/2026-11/
//! 02-range-calibration.md`, option 4).
//!
//! The venue omits mucked hands from `shown_cards`, so showdowns alone skew strong. A seat that
//! reached the showdown without showing is kept: its pair is hidden, but it must have lost.

use svanbot_core::card::CardSet;
use svanbot_core::eval::{Strength, strength};
use svanbot_strategy::action::{
    ActionModel, BUCKETS, Counts, Move, Seen, Situation, buckets, combos,
};

/// The least held-out gain, in nats per hand, a fit's lower bound must clear: v10's gate.
pub const MARGIN: f64 = 0.005;

/// The one-sided 95% normal quantile.
const Z95: f64 = 1.645;

/// What a hand showed of one opponent's cards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The hand ended without them: a fold, theirs or everyone else's.
    Unseen,
    /// They showed this pair.
    Shown(CardSet),
    /// They reached the showdown on `board` and mucked: their pair lost to `winner`.
    Mucked { board: CardSet, winner: Strength },
}

/// One opponent in one hand, as the fit takes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observed {
    /// The hand, so that the gate counts opponents in one hand as one sample.
    pub hand: u64,
    pub seen: Vec<Seen>,
    /// Cards the opponent cannot hold: the board, our own cards, cards others showed.
    pub dead: CardSet,
    pub outcome: Outcome,
}

/// An observation made ready for the fit: each candidate pair's bucket at every action.
#[derive(Clone, Debug)]
pub struct Prepared {
    hand: u64,
    /// How many pairs the opponent could hold before the outcome rules any out.
    live: usize,
    /// How many pairs the outcome leaves.
    candidates: usize,
    /// Each action, with every candidate's bucket at it.
    steps: Vec<(Situation, Move, Vec<u8>)>,
}

impl Prepared {
    /// How many pairs the opponent could have held, given what the hand showed.
    pub fn candidates(&self) -> usize {
        self.candidates
    }

    /// Each candidate's weight: the chance it takes every action seen.
    fn weights(&self, model: &ActionModel) -> Vec<f64> {
        let mut weights = vec![1.0; self.candidates];
        for (situation, mv, at) in &self.steps {
            for (weight, &bucket) in weights.iter_mut().zip(at) {
                *weight *= model.prob(*situation, usize::from(bucket), *mv);
            }
        }
        weights
    }
}

/// Makes `observed` ready for the fit.
pub fn prepare(observed: &Observed) -> Prepared {
    let boards = observed
        .seen
        .iter()
        .fold(CardSet::EMPTY, |cards, step| cards | step.board);
    let live = combos(observed.dead | boards);
    let candidates: Vec<CardSet> = match &observed.outcome {
        Outcome::Unseen => live.clone(),
        Outcome::Shown(pair) => live.iter().copied().filter(|c| c == pair).collect(),
        Outcome::Mucked { board, winner } => live
            .iter()
            .copied()
            .filter(|&pair| strength(pair | *board) < *winner)
            .collect(),
    };
    let steps = observed
        .seen
        .iter()
        .map(|step| {
            // Both lists come from `combos` in one order, and `ranked` holds every candidate.
            let ranked = buckets(step.board, observed.dead);
            let mut at = 0;
            let bucket = candidates
                .iter()
                .map(|pair| {
                    while ranked[at].0 != *pair {
                        at += 1;
                    }
                    ranked[at].1 as u8
                })
                .collect();
            (step.situation, step.mv, bucket)
        })
        .collect();
    Prepared {
        hand: observed.hand,
        live: live.len(),
        candidates: candidates.len(),
        steps,
    }
}

/// The log-likelihood of what `prepared` saw under `model`: the log of the chance, summed over
/// every pair the opponent could hold, that the pair acts as seen and the outcome follows.
pub fn log_likelihood(model: &ActionModel, prepared: &Prepared) -> f64 {
    let total: f64 = prepared.weights(model).iter().sum();
    (total / prepared.live.max(1) as f64).ln()
}

/// A fitted model, and the training log-likelihood before each round.
#[derive(Clone, Debug)]
pub struct Fit {
    pub model: ActionModel,
    pub trace: Vec<f64>,
}

/// Fits the action model to `train` by `rounds` rounds of EM from the uniform model. `alpha` is
/// the Dirichlet prior on each move's count.
pub fn fit(train: &[Prepared], rounds: usize, alpha: f64) -> Fit {
    let mut model = ActionModel::uniform();
    let mut trace = Vec::with_capacity(rounds + 1);
    for _ in 0..rounds {
        let mut counts = Counts::default();
        let mut total = 0.0;
        for prepared in train {
            let weights = prepared.weights(&model);
            let sum: f64 = weights.iter().sum();
            if sum <= 0.0 {
                continue;
            }
            total += (sum / prepared.live.max(1) as f64).ln();
            for (situation, mv, at) in &prepared.steps {
                let mut per_bucket = [0.0; BUCKETS];
                for (weight, &bucket) in weights.iter().zip(at) {
                    per_bucket[usize::from(bucket)] += weight / sum;
                }
                for (bucket, share) in per_bucket.into_iter().enumerate() {
                    if share > 0.0 {
                        counts.add(*situation, bucket, *mv, share);
                    }
                }
            }
        }
        trace.push(total);
        model = ActionModel::from_counts(&counts, alpha);
    }
    trace.push(train.iter().map(|p| log_likelihood(&model, p)).sum());
    Fit { model, trace }
}

/// A gate's verdict on a challenger against the incumbent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    /// Held-out hands compared.
    pub hands: usize,
    /// The mean gain in log-likelihood per hand, in nats.
    pub gain: f64,
    /// The one-sided 95% lower bound on the gain.
    pub lower: f64,
    /// Whether the lower bound clears the margin.
    pub passed: bool,
}

/// Whether `challenger` beats `incumbent` on `held_out`: the paired gain in log-likelihood,
/// summed per hand so the opponents in one hand count as one sample, with a 95% lower bound that
/// must clear `margin`.
pub fn gate(
    challenger: &ActionModel,
    incumbent: &ActionModel,
    held_out: &[Prepared],
    margin: f64,
) -> Verdict {
    let mut per_hand: Vec<(u64, f64)> = Vec::new();
    for prepared in held_out {
        let gain = log_likelihood(challenger, prepared) - log_likelihood(incumbent, prepared);
        match per_hand.last_mut() {
            Some((hand, sum)) if *hand == prepared.hand => *sum += gain,
            _ => per_hand.push((prepared.hand, gain)),
        }
    }
    let n = per_hand.len();
    if n < 2 {
        return Verdict {
            hands: n,
            gain: 0.0,
            lower: f64::NEG_INFINITY,
            passed: false,
        };
    }
    let mean = per_hand.iter().map(|(_, g)| g).sum::<f64>() / n as f64;
    let variance = per_hand
        .iter()
        .map(|(_, g)| (g - mean).powi(2))
        .sum::<f64>()
        / (n - 1) as f64;
    let lower = mean - Z95 * (variance / n as f64).sqrt();
    Verdict {
        hands: n,
        gain: mean,
        lower,
        passed: lower > margin,
    }
}
