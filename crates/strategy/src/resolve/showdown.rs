//! Showdown values over whole ranges at once, in time linear in the range: the pairs sorted by
//! strength once, then one sweep per call, with card removal by inclusion and exclusion over the
//! two cards of each pair.

use svanbot_core::card::CardSet;
use svanbot_core::eval::{Strength, strength};

use crate::action::BUCKETS;

/// The pairs of a range and how they rank on a board.
#[derive(Clone, Debug)]
pub struct Showdown {
    /// Each pair's two cards, as 0..52.
    cards: Vec<[usize; 2]>,
    /// Pair indices grouped by equal strength, weakest group first. A pair that shares a card
    /// with the board is in no group: it cannot be held.
    groups: Vec<Vec<usize>>,
    /// Whether each pair can be held on the board.
    live: Vec<bool>,
    /// Each live pair's strength bucket on the board, as the action model cuts them.
    bucket: Vec<u8>,
}

/// A card's number, 0..52.
pub fn index(card: svanbot_core::card::Card) -> usize {
    card.rank() as usize * 4 + card.suit() as usize
}

impl Showdown {
    /// Ranks `pairs` on `board` (three to five cards). A pair that shares a card with the board
    /// is dead there: it has no reach and no value.
    pub fn new(pairs: &[CardSet], board: CardSet) -> Self {
        let live: Vec<bool> = pairs.iter().map(|pair| pair.is_disjoint(board)).collect();
        let cards: Vec<[usize; 2]> = pairs
            .iter()
            .map(|pair| {
                let mut it = pair.iter().map(index);
                [it.next().unwrap_or(0), it.next().unwrap_or(0)]
            })
            .collect();
        let mut ranked: Vec<(Strength, usize)> = pairs
            .iter()
            .enumerate()
            .filter(|&(at, _)| live[at])
            .map(|(at, &pair)| (strength(pair | board), at))
            .collect();
        ranked.sort_unstable();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut last = None;
        for (value, at) in ranked {
            if last == Some(value) {
                groups.last_mut().expect("a group is open").push(at);
            } else {
                groups.push(vec![at]);
                last = Some(value);
            }
        }
        let total: usize = groups.iter().map(Vec::len).sum();
        let mut bucket = vec![0; pairs.len()];
        let mut start = 0;
        for group in &groups {
            // Equal strengths share a bucket, from the middle of their run.
            let share = (2 * start + group.len()) as f64 / 2.0 / total.max(1) as f64;
            let at = ((share * BUCKETS as f64) as usize).min(BUCKETS - 1);
            for &pair in group {
                bucket[pair] = at as u8;
            }
            start += group.len();
        }
        Self {
            cards,
            groups,
            live,
            bucket,
        }
    }

    /// For each pair, against the other side's `reach` over the same pairs: the reach it beats
    /// plus half the reach it ties, and the reach that shares no card with it.
    pub fn against(&self, reach: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let mut share = vec![0.0; self.cards.len()];
        let compatible = self.compatible(reach);
        let mut below = 0.0;
        let mut below_card = [0.0; 52];
        for group in &self.groups {
            let mut tied = 0.0;
            let mut tied_card = [0.0; 52];
            for &at in group {
                let [a, b] = self.cards[at];
                tied += reach[at];
                tied_card[a] += reach[at];
                tied_card[b] += reach[at];
            }
            for &at in group {
                let [a, b] = self.cards[at];
                let beaten = below - below_card[a] - below_card[b];
                // Inclusion and exclusion: the pair itself is in both card sums, so adding its
                // reach back once leaves it out, as it must be: it shares both cards.
                let ties = tied - tied_card[a] - tied_card[b] + reach[at];
                share[at] = beaten + 0.5 * ties;
            }
            below += tied;
            for (sum, add) in below_card.iter_mut().zip(tied_card) {
                *sum += add;
            }
        }
        (share, compatible)
    }

    /// Each pair's strength bucket on the board. A dead pair has none, and reads 0.
    pub fn bucket(&self) -> &[u8] {
        &self.bucket
    }

    /// [`Showdown::against`] for `M` reaches at once, one per column, in one sweep.
    pub fn against_wide<const M: usize>(
        &self,
        reach: &[[f64; M]],
    ) -> (Vec<[f64; M]>, Vec<[f64; M]>) {
        let mut share = vec![[0.0; M]; self.cards.len()];
        let mut compatible = vec![[0.0; M]; self.cards.len()];
        let mut below = [0.0; M];
        let mut below_card = [[0.0; M]; 52];
        for group in &self.groups {
            // The reach strictly below the group, before the group is added.
            for &at in group {
                let [a, b] = self.cards[at];
                for m in 0..M {
                    share[at][m] = below[m] - below_card[a][m] - below_card[b][m];
                }
            }
            for &at in group {
                let [a, b] = self.cards[at];
                for m in 0..M {
                    below[m] += reach[at][m];
                    below_card[a][m] += reach[at][m];
                    below_card[b][m] += reach[at][m];
                }
            }
            // With the group added the same sum takes in the ties: half of them is the share.
            for &at in group {
                let [a, b] = self.cards[at];
                for m in 0..M {
                    let tied_in = below[m] - below_card[a][m] - below_card[b][m] + reach[at][m];
                    share[at][m] = 0.5 * (share[at][m] + tied_in);
                }
            }
        }
        for (at, &[a, b]) in self.cards.iter().enumerate() {
            if self.live[at] {
                for m in 0..M {
                    compatible[at][m] =
                        below[m] - below_card[a][m] - below_card[b][m] + reach[at][m];
                }
            }
        }
        (share, compatible)
    }

    /// Whether each pair can be held on the board.
    pub fn live(&self) -> &[bool] {
        &self.live
    }

    /// For each pair, the reach on the other side that shares no card with it.
    pub fn compatible(&self, reach: &[f64]) -> Vec<f64> {
        let mut total = 0.0;
        let mut per_card = [0.0; 52];
        for (at, &[a, b]) in self.cards.iter().enumerate() {
            if !self.live[at] {
                continue;
            }
            total += reach[at];
            per_card[a] += reach[at];
            per_card[b] += reach[at];
        }
        self.cards
            .iter()
            .enumerate()
            .map(|(at, &[a, b])| match self.live[at] {
                true => total - per_card[a] - per_card[b] + reach[at],
                false => 0.0,
            })
            .collect()
    }
}
