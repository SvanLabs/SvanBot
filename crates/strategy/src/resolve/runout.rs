//! The run-outs a street's end is valued on: ways the rest of the board can come.
//!
//! A river has one, the board itself. Earlier, a fixed number is drawn from the cards not on the
//! board, by seed. The cards of the run-outs are all different, so they spread over the deck: a
//! flush draw meets its share of the suit.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::rng::Rng;

/// The most run-outs a street's end is valued on.
pub const RUNOUTS: usize = 16;

/// Full boards that `board` (three to five cards) can run out to, drawn from `rng`.
pub fn runouts(board: CardSet, rng: &mut Rng) -> Vec<CardSet> {
    let missing = 5usize.saturating_sub(board.len());
    if missing == 0 {
        return vec![board];
    }
    let mut deck: Vec<Card> = Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .filter(|&card| !board.contains(card))
        .collect();
    let count = RUNOUTS.min(deck.len() / missing);
    (0..count)
        .map(|_| {
            let mut full = board;
            for _ in 0..missing {
                full.insert(deck.swap_remove(rng.below(deck.len() as u64) as usize));
            }
            full
        })
        .collect()
}
