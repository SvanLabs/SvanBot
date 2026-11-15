//! The pots and who wins them. Each level a seat still in the hand put in, from the lowest, makes
//! a pot that every seat which reached it shares in. Seats that folded feed the pots but win none.
//! A bet nobody called comes back as a pot only its bettor reached, and chips above every seat
//! still in come back to whoever put them in.

use super::Hand;
use crate::eval::Strength;

impl Hand {
    /// The chips each seat takes from the pot once the hand is over. `strength` gives the
    /// strength of a seat still in the hand at the showdown. It is never asked of a seat that
    /// folded, nor at all when one seat is left. A pot that splits unevenly gives its odd chips
    /// one each to its winners, from the first left of the button.
    pub fn payouts(&self, strength: impl Fn(usize) -> Strength) -> Vec<u32> {
        let n = self.seats.len();
        let in_hand: Vec<usize> = (0..n).filter(|&i| !self.seats[i].folded).collect();
        let strengths: Vec<Option<Strength>> = (0..n)
            .map(|i| (in_hand.len() > 1 && !self.seats[i].folded).then(|| strength(i)))
            .collect();
        let mut levels: Vec<u32> = self.seats.iter().map(|seat| seat.total).collect();
        levels.sort_unstable();
        levels.dedup();

        let mut payouts = vec![0; n];
        let mut below = 0;
        for level in levels {
            let shares: Vec<u32> = self
                .seats
                .iter()
                .map(|seat| seat.total.min(level) - seat.total.min(below))
                .collect();
            below = level;
            let reached: Vec<usize> = in_hand
                .iter()
                .copied()
                .filter(|&i| self.seats[i].total >= level)
                .collect();
            if reached.is_empty() {
                // Only seats that folded reached this level, as when the small blind folds to a
                // big blind all in for less: their chips come back.
                for (paid, share) in payouts.iter_mut().zip(shares) {
                    *paid += share;
                }
                continue;
            }
            let pot: u32 = shares.iter().sum();
            let best = reached.iter().map(|&i| strengths[i]).max().flatten();
            // From the first seat left of the button, round the table.
            let mut winners: Vec<usize> = reached
                .into_iter()
                .filter(|&i| strengths[i] == best)
                .collect();
            winners.sort_by_key(|&i| (i + n - self.button - 1) % n);
            let count = winners.len() as u32;
            for (k, &winner) in winners.iter().enumerate() {
                payouts[winner] += pot / count + u32::from((k as u32) < pot % count);
            }
        }
        payouts
    }
}
