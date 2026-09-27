//! Decision grades (0220): chess-style review for poker decisions, used by the dashboard's
//! accuracy panel (the analyst's deep re-solves graded against the live choice) and the "What would
//! Svanbot do?" quiz (a player's pick graded against the bot's best candidate).
//!
//! A decision's loss is the EV of the best option minus the EV of the chosen one, in chips. It is
//! graded as a share of the pot, so a 5 bb loss is a blunder in a 10 bb pot and an inaccuracy in a
//! 100 bb pot; a loss under 0.2 bb is always Best (research round 2: GTO Wizard grades by EV loss,
//! chess.com by expected-points loss). Accuracy per decision follows Lichess's formula with the pot
//! share in percent standing in for the win-percentage drop: `103.1668·e^(−0.04354·loss%) − 3.1669`,
//! clamped to 0..100.

use serde::Serialize;

/// A decision's grade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    /// Within 2% of the pot (or 0.2 bb) of the best option.
    Best,
    /// Within 10% of the pot.
    Good,
    /// Within 20% of the pot.
    Inaccuracy,
    /// Within 40% of the pot.
    Mistake,
    /// Losing 40% of the pot or more.
    Blunder,
}

impl Grade {
    /// All grades, best first.
    pub const ALL: [Grade; 5] = [Grade::Best, Grade::Good, Grade::Inaccuracy, Grade::Mistake, Grade::Blunder];
}

/// Share of the pot below which a loss is graded Best, then Good, Inaccuracy and Mistake.
const BANDS: [f64; 4] = [0.02, 0.10, 0.20, 0.40];
/// A loss this small (big blinds) is Best whatever the pot.
const BEST_BB: f64 = 0.2;

/// Grade a loss of `loss_bb` big blinds in a `pot_bb` pot.
pub fn grade(loss_bb: f64, pot_bb: f64) -> Grade {
    let loss = loss_bb.max(0.0);
    if loss < BEST_BB {
        return Grade::Best;
    }
    let share = loss / pot_bb.max(1.0);
    match BANDS.iter().position(|&b| share < b) {
        Some(0) => Grade::Best,
        Some(1) => Grade::Good,
        Some(2) => Grade::Inaccuracy,
        Some(3) => Grade::Mistake,
        _ => Grade::Blunder,
    }
}

/// Accuracy (0..100) of one decision losing `loss_bb` in a `pot_bb` pot.
pub fn accuracy(loss_bb: f64, pot_bb: f64) -> f64 {
    if loss_bb.max(0.0) < BEST_BB {
        return 100.0;
    }
    let pct = 100.0 * loss_bb / pot_bb.max(1.0);
    (103.1668 * (-0.04354 * pct).exp() - 3.1669).clamp(0.0, 100.0)
}

/// Totals over a set of graded decisions.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    /// Decisions graded.
    pub decisions: usize,
    /// Mean accuracy (0..100).
    pub accuracy: f64,
    /// Decisions per grade, best first.
    pub grades: [usize; 5],
    /// Mean loss per decision, big blinds.
    pub mean_loss_bb: f64,
}

/// Grade every (loss_bb, pot_bb) pair.
pub fn report(losses: impl IntoIterator<Item = (f64, f64)>) -> Report {
    let mut r = Report::default();
    let (mut acc, mut loss) = (0.0, 0.0);
    for (l, pot) in losses {
        r.decisions += 1;
        acc += accuracy(l, pot);
        loss += l.max(0.0);
        let g = grade(l, pot);
        r.grades[Grade::ALL.iter().position(|x| *x == g).unwrap_or(0)] += 1;
    }
    if r.decisions > 0 {
        r.accuracy = acc / r.decisions as f64;
        r.mean_loss_bb = loss / r.decisions as f64;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_follow_the_share_of_the_pot() {
        assert_eq!(grade(0.0, 50.0), Grade::Best);
        assert_eq!(grade(0.15, 1.0), Grade::Best, "tiny losses are always best");
        assert_eq!(grade(0.9, 100.0), Grade::Best);
        assert_eq!(grade(5.0, 100.0), Grade::Good);
        assert_eq!(grade(15.0, 100.0), Grade::Inaccuracy);
        assert_eq!(grade(30.0, 100.0), Grade::Mistake);
        assert_eq!(grade(5.0, 10.0), Grade::Blunder, "the same 5 bb in a small pot");
        assert_eq!(grade(-3.0, 10.0), Grade::Best, "a negative loss is no loss");
    }

    #[test]
    fn accuracy_is_100_for_best_play_and_falls_with_the_loss() {
        assert_eq!(accuracy(0.0, 20.0), 100.0);
        let a = accuracy(10.0, 100.0);
        assert!((a - (103.1668 * (-0.4354f64).exp() - 3.1669)).abs() < 1e-9);
        assert!(accuracy(20.0, 100.0) < a && accuracy(500.0, 100.0) == 0.0);
    }

    #[test]
    fn a_report_counts_grades_and_averages() {
        let r = report([(0.0, 10.0), (5.0, 10.0), (15.0, 100.0)]);
        assert_eq!(r.decisions, 3);
        assert_eq!(r.grades, [1, 0, 1, 0, 1]);
        assert!((r.mean_loss_bb - 20.0 / 3.0).abs() < 1e-9);
        assert!(r.accuracy > 0.0 && r.accuracy < 100.0);
        assert_eq!(report(std::iter::empty()), Report::default());
    }
}
