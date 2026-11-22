//! The ring: one deal played once from every seat of a full table, so the challenger holds every
//! hand in every position against the same opponents.

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::paired::Match;
use svanbot_learn::ring::{SIX_MAX, ring};

const MATCH: Match = Match {
    blinds: Blinds { small: 10, big: 20 },
    stack: 2000,
    seed: 11,
};

fn folds(_: &Spot, _: u64) -> Action {
    Action::Fold
}

/// A policy that leans on its seed, so a ring that handed the seats different seeds from one
/// rotation to the next would play different hands and stop breaking even.
fn mixes(spot: &Spot, seed: u64) -> Action {
    match (seed % 3, spot.legal.raise, spot.legal.call) {
        (0, Some(range), _) => Action::RaiseTo(range.min),
        (_, _, Some(_)) => Action::Call,
        _ => Action::Check,
    }
}

#[test]
fn a_policy_against_itself_breaks_even_on_every_deal() {
    // Every rotation plays the same hand, and the challenger is in a different seat each time:
    // over the six it has won what the whole table won, which is nothing.
    for number in 0..40 {
        let won = ring(&MATCH, SIX_MAX, number, &mixes, &|_, spot, seed| {
            mixes(spot, seed)
        });
        assert!(won.abs() < 1e-9, "deal {number} (seed 11): {won}");
    }
}

#[test]
fn folding_against_folding_breaks_even() {
    // Each rotation the big blind takes the small: the challenger posts each blind once.
    let won = ring(&MATCH, SIX_MAX, 0, &folds, &|_, spot, seed| {
        folds(spot, seed)
    });
    assert!(won.abs() < 1e-12, "{won}");
}

#[test]
fn a_ring_repeats_for_its_seed_and_differs_by_its_number() {
    let opponent = |_: usize, spot: &Spot, _: u64| match spot.legal.call {
        Some(_) => Action::Call,
        None => Action::Check,
    };
    let first: Vec<f64> = (0..8)
        .map(|n| ring(&MATCH, SIX_MAX, n, &mixes, &opponent))
        .collect();
    let again: Vec<f64> = (0..8)
        .map(|n| ring(&MATCH, SIX_MAX, n, &mixes, &opponent))
        .collect();
    assert_eq!(first, again);
    assert!(first.windows(2).any(|pair| pair[0] != pair[1]), "{first:?}");
}
