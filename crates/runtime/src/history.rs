//! Stored hands made ready for the fit: each opponent's side of a hand as a
//! [`learn::fit::Observed`](svanbot_learn::fit::Observed).
//!
//! An opponent who showed gives their pair. One who reached the showdown and did not show mucked,
//! so their pair lost to the best hand shown there. Anyone else's pair is unseen. Our own seat is
//! never an opponent.

use svanbot_core::card::CardSet;
use svanbot_core::eval::strength;
use svanbot_learn::fit::{Observed, Outcome};
use svanbot_strategy::action::Seen;

use crate::lines::Lines;
use crate::record::Hand;
use crate::wire::{Inbound, parse};

/// The opponents' sides of `hand`, numbered `number` so the gate can count them as one hand.
/// Frames that do not parse are skipped.
pub fn observed(hand: &Hand, number: u64) -> Vec<Observed> {
    let mut lines = Lines::default();
    let mut me = None;
    let mut hole = CardSet::EMPTY;
    let mut shown: Vec<(usize, CardSet)> = Vec::new();
    for text in &hand.frames {
        let Ok(frame) = parse(text) else {
            continue;
        };
        match &frame {
            Inbound::HandStart { seat, .. } => me = Some(*seat),
            Inbound::HoleCards { cards } => hole = cards.iter().copied().collect(),
            Inbound::HandResult { shown: at, .. } => shown.clone_from(at),
            _ => {}
        }
        lines.on(&frame);
    }
    let board = lines.board();
    let showdown = board.len() == 5 && !shown.is_empty();
    // The best hand at the showdown: the best shown, or ours if we were in it.
    let mut best = shown.iter().map(|&(_, pair)| strength(pair | board)).max();
    if showdown && me.is_some_and(|seat| !lines.folded(seat)) && hole.len() == 2 {
        best = best.max(Some(strength(hole | board)));
    }
    lines
        .seats()
        .filter(|&seat| Some(seat) != me)
        .map(|seat| {
            let own = shown
                .iter()
                .find(|&&(at, _)| at == seat)
                .map(|&(_, pair)| pair);
            let outcome = match (own, showdown && !lines.folded(seat), best) {
                (Some(pair), _, _) => Outcome::Shown(pair),
                (None, true, Some(winner)) => Outcome::Mucked { board, winner },
                _ => Outcome::Unseen,
            };
            let others_shown = shown
                .iter()
                .filter(|&&(at, _)| at != seat)
                .fold(CardSet::EMPTY, |cards, &(_, pair)| cards | pair);
            Observed {
                hand: number,
                seen: lines.of(seat).iter().map(Seen::of).collect(),
                dead: hole | board | others_shown,
                outcome,
            }
        })
        .collect()
}
