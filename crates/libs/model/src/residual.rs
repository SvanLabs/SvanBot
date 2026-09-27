//! Per-opponent residual correction for the neural response model (0210).
//!
//! The network reads an opponent through shrunk summary statistics, so a player whose choices the
//! statistics do not capture is mispredicted the same way hand after hand. This table keeps, per
//! player and per situation (facing a bet or not), how often each response class happened (`O`)
//! against how often the network expected it (`E`, the sum of its probabilities), and scales the
//! network's probability of each class by the shrunk ratio `(O + prior) / (E + prior)` before
//! renormalizing over the legal classes. A player with no history keeps the network's prediction.

use std::collections::HashMap;

/// Observed and expected counts per response class for one player in one situation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Cell {
    observed: [f32; 3],
    expected: [f32; 3],
}

/// Shrunk observed/expected ratios per player, situation and response class.
#[derive(Clone, Debug, Default)]
pub struct ResidualTable {
    prior: f32,
    cells: HashMap<(String, bool), Cell>,
}

impl ResidualTable {
    /// An empty table whose ratios are shrunk toward 1 with `prior` pseudo-observations per class.
    pub fn new(prior: f32) -> ResidualTable {
        ResidualTable { prior: prior.max(0.0), cells: HashMap::new() }
    }

    /// Players tracked.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether no player is tracked.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// `player`'s shrunk observed/expected ratio per response class in one situation, if tracked.
    pub fn ratio(&self, player: &str, facing: bool) -> Option<[f32; 3]> {
        let cell = self.cells.get(&(player.to_string(), facing))?;
        Some([0, 1, 2].map(|c| (cell.observed[c] + self.prior) / (cell.expected[c] + self.prior)))
    }

    /// Every tracked player's ratios when facing a bet (the decisions the fleet prices).
    pub fn facing_ratios(&self) -> HashMap<String, [f32; 3]> {
        self.cells.keys().filter(|(_, facing)| *facing).filter_map(|(name, _)| Some((name.clone(), self.ratio(name, true)?))).collect()
    }

    /// The network's `probs` for `player` corrected by their residuals (legal classes per `mask`).
    pub fn adjust(&self, player: &str, facing: bool, probs: &[f32], mask: &[bool]) -> Vec<f32> {
        match self.ratio(player, facing) {
            Some(ratio) => apply_ratio(probs, mask, ratio),
            None => probs.to_vec(),
        }
    }

    /// Record that `player` chose `label` where the network predicted `probs` (its uncorrected output).
    pub fn observe(&mut self, player: &str, facing: bool, probs: &[f32], mask: &[bool], label: usize) {
        let cell = self.cells.entry((player.to_string(), facing)).or_default();
        for (c, (&p, &legal)) in probs.iter().zip(mask).enumerate().take(3) {
            if legal && p.is_finite() {
                cell.expected[c] += p;
            }
        }
        if label < 3 {
            cell.observed[label] += 1.0;
        }
    }
}

/// No correction: every class keeps the network's probability.
pub const UNIT_RATIO: [f32; 3] = [1.0; 3];

/// Scale `probs` by `ratio` per class and renormalize over the legal classes (`mask`); the
/// prediction is returned unchanged if the result is not a distribution.
pub fn apply_ratio(probs: &[f32], mask: &[bool], ratio: [f32; 3]) -> Vec<f32> {
    if ratio == UNIT_RATIO {
        return probs.to_vec();
    }
    let mut out: Vec<f32> =
        probs.iter().zip(mask).enumerate().map(|(c, (&p, &legal))| if legal && c < 3 { p * ratio[c] } else { 0.0 }).collect();
    let total: f32 = out.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        return probs.to_vec();
    }
    out.iter_mut().for_each(|p| *p /= total);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FACING: [bool; 3] = [true, true, true];

    #[test]
    fn an_unknown_player_keeps_the_network_prediction() {
        let t = ResidualTable::new(30.0);
        assert_eq!(t.adjust("nobody", true, &[0.5, 0.4, 0.1], &FACING), vec![0.5, 0.4, 0.1]);
    }

    #[test]
    fn a_player_who_folds_more_than_predicted_is_read_as_folding_more() {
        let mut t = ResidualTable::new(30.0);
        for _ in 0..300 {
            t.observe("nit", true, &[0.5, 0.4, 0.1], &FACING, 0);
        }
        let p = t.adjust("nit", true, &[0.5, 0.4, 0.1], &FACING);
        assert!(p[0] > 0.8, "{p:?}");
        assert_eq!(t.facing_ratios().len(), 1);
        assert_eq!(apply_ratio(&[0.5, 0.4, 0.1], &FACING, t.ratio("nit", true).unwrap()), p);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        // The other situation and other players are untouched.
        assert_eq!(t.adjust("nit", false, &[0.0, 0.7, 0.3], &[false, true, true]), vec![0.0, 0.7, 0.3]);
        assert_eq!(t.adjust("other", true, &[0.5, 0.4, 0.1], &FACING), vec![0.5, 0.4, 0.1]);
    }

    #[test]
    fn a_few_hands_move_the_prediction_only_a_little_and_illegal_classes_stay_zero() {
        let mut t = ResidualTable::new(30.0);
        for _ in 0..3 {
            t.observe("new", false, &[0.0, 0.7, 0.3], &[false, true, true], 2);
        }
        let p = t.adjust("new", false, &[0.0, 0.7, 0.3], &[false, true, true]);
        assert_eq!(p[0], 0.0);
        assert!(p[2] > 0.3 && p[2] < 0.36, "{p:?}");
    }

    #[test]
    fn a_calibrated_player_is_left_alone() {
        let mut t = ResidualTable::new(30.0);
        for i in 0..1000 {
            t.observe("fair", true, &[0.5, 0.4, 0.1], &FACING, [0, 0, 0, 0, 0, 1, 1, 1, 1, 2][i % 10]);
        }
        let p = t.adjust("fair", true, &[0.5, 0.4, 0.1], &FACING);
        for (a, b) in p.iter().zip([0.5, 0.4, 0.1]) {
            assert!((a - b).abs() < 0.01, "{p:?}");
        }
    }
}
