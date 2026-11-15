//! The drift monitor: is the range still calibrated, checked after every showdown, and valid
//! however long it runs.
//!
//! At each showdown the shown pair's place in the predicted range is its PIT: the share of the
//! range weaker than it, plus a random share of the ties. A calibrated range gives PITs that are
//! uniform on `[0, 1)`. The monitor bins them and runs a mixture likelihood ratio against the
//! uniform: the Krichevsky–Trofimov estimate of the bins' chances, over `1/BINS` each. That
//! product is a nonnegative martingale with mean 1 while the range is calibrated, so by Ville's
//! inequality it reaches `1/alpha` with chance at most `alpha`, at any stopping time. Crossing it
//! is drift: the learner refits or rolls back (`docs/research/2026-11/02-range-calibration.md`,
//! option 2). The histogram is kept whole, because a mean PIT of 0.5 can hide a U shape.

use svanbot_core::card::CardSet;
use svanbot_core::eval::strength;

/// Bins of the PIT histogram.
pub const BINS: usize = 10;

/// The PIT of `shown` on `board` within `range`: the share of the range weaker than it, plus
/// `u` (drawn uniformly from `[0, 1)` by the caller) times the share that ties it.
pub fn pit(range: &[(CardSet, f64)], shown: CardSet, board: CardSet, u: f64) -> f64 {
    let target = strength(shown | board);
    let (mut weaker, mut equal, mut total) = (0.0, 0.0, 0.0);
    for &(pair, weight) in range {
        if !pair.is_disjoint(board) {
            continue;
        }
        total += weight;
        match strength(pair | board).cmp(&target) {
            std::cmp::Ordering::Less => weaker += weight,
            std::cmp::Ordering::Equal => equal += weight,
            std::cmp::Ordering::Greater => {}
        }
    }
    if total <= 0.0 {
        return u;
    }
    ((weaker + u * equal) / total).clamp(0.0, 1.0)
}

/// The monitor for one model's ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct Drift {
    bins: [u64; BINS],
    log_e: f64,
    alpha: f64,
}

impl Drift {
    /// A monitor that alarms at false-alarm rate `alpha`.
    pub fn new(alpha: f64) -> Self {
        Self {
            bins: [0; BINS],
            log_e: 0.0,
            alpha,
        }
    }

    /// Takes one PIT, and says whether the monitor is now alarmed.
    pub fn observe(&mut self, pit: f64) -> bool {
        let bin = ((pit * BINS as f64) as usize).min(BINS - 1);
        let seen: u64 = self.bins.iter().sum();
        // The Krichevsky–Trofimov chance of this bin, from the bins before it.
        let predicted = (self.bins[bin] as f64 + 0.5) / (seen as f64 + BINS as f64 / 2.0);
        self.log_e += (predicted * BINS as f64).ln();
        self.bins[bin] += 1;
        self.alarmed()
    }

    /// Whether the evidence against calibration has reached `1/alpha`.
    pub fn alarmed(&self) -> bool {
        self.log_e >= (1.0 / self.alpha).ln()
    }

    /// The e-value so far: how many times more likely the PITs are under drift than under a
    /// calibrated range.
    pub fn e_value(&self) -> f64 {
        self.log_e.exp()
    }

    /// The PIT histogram, lowest bin first.
    pub fn histogram(&self) -> [u64; BINS] {
        self.bins
    }
}
