//! Showdown values over whole ranges at once, in time linear in the range: the pairs sorted by
//! strength once, then one sweep per call, with card removal by inclusion and exclusion over the
//! two cards of each pair.

use svanbot_core::card::CardSet;
use svanbot_core::eval::{Strength, strength};

/// The pairs of a range and how they rank on a board.
#[derive(Clone, Debug)]
pub struct Showdown {
    /// Each pair's two cards, as 0..52.
    cards: Vec<[usize; 2]>,
    /// Pair indices grouped by equal strength, weakest group first.
    groups: Vec<Vec<usize>>,
}

/// A card's number, 0..52.
pub fn index(card: svanbot_core::card::Card) -> usize {
    card.rank() as usize * 4 + card.suit() as usize
}

impl Showdown {
    /// Ranks `pairs` on `board` (three to five cards).
    pub fn new(pairs: &[CardSet], board: CardSet) -> Self {
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
        Self { cards, groups }
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

    /// For each pair, the reach on the other side that shares no card with it.
    pub fn compatible(&self, reach: &[f64]) -> Vec<f64> {
        let mut total = 0.0;
        let mut per_card = [0.0; 52];
        for (at, &[a, b]) in self.cards.iter().enumerate() {
            total += reach[at];
            per_card[a] += reach[at];
            per_card[b] += reach[at];
        }
        self.cards
            .iter()
            .enumerate()
            .map(|(at, &[a, b])| total - per_card[a] - per_card[b] + reach[at])
            .collect()
    }
}
