//! The paired sim: two policies play the same deals from mirrored seats, heads up.

use svanbot_core::card::Rank;
use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::paired::{Match, duel, duel_with, verdict};

const MATCH: Match = Match {
    blinds: Blinds { small: 10, big: 20 },
    stack: 2000,
    seed: 9,
};

fn folds(_: &Spot, _: u64) -> Action {
    Action::Fold
}

fn shoves(spot: &Spot, _: u64) -> Action {
    Action::RaiseTo(spot.legal.raise.map_or(0, |range| range.max))
}

#[test]
fn both_hands_of_a_deal_face_the_same_opponent_state() {
    // A learning opponent remembers our bets, so the second hand of a deal would otherwise meet
    // one already adapted by the first, and the deal would no longer cancel card luck. The reset
    // runs before each hand: what it puts back is what both hands see.
    use std::cell::Cell;
    let resets = Cell::new(0);
    let result = duel_with(&MATCH, 3, &calls_good_cards, &folds, || {
        resets.set(resets.get() + 1)
    });
    assert_eq!(resets.get(), 2, "one reset before each mirrored hand");
    // The hook changes no deal: it is the same duel, played from the same streams.
    assert_eq!(result, duel(&MATCH, 3, &calls_good_cards, &folds));
}

/// Calls with a pair in the hole or an ace, and gives up otherwise.
fn calls_good_cards(spot: &Spot, _: u64) -> Action {
    let ranks: Vec<_> = spot.hole.iter().map(|card| card.rank()).collect();
    let good = ranks[0] == ranks[1] || ranks.contains(&Rank::Ace);
    match (good, spot.legal.call) {
        (true, Some(_)) => Action::Call,
        (true, None) => Action::Check,
        (false, _) => Action::Fold,
    }
}

#[test]
fn shoving_into_a_seat_that_always_folds_wins_both_blinds_over_the_pair_of_hands() {
    // On the button the shove takes the big blind, 20. In the big blind the fold gives it the
    // small blind, 10. That is 30 chips over two hands: 0.75 big blinds a hand.
    for deal in 0..20 {
        assert_eq!(duel(&MATCH, deal, &shoves, &folds), 0.75);
        assert_eq!(duel(&MATCH, deal, &folds, &shoves), -0.75);
    }
}

#[test]
fn a_policy_against_itself_breaks_even_on_every_deal() {
    for deal in 0..50 {
        // Up to the rounding of the corrected results.
        assert!(duel(&MATCH, deal, &calls_good_cards, &calls_good_cards).abs() < 1e-9);
    }
}

#[test]
fn the_cards_decide_a_deal_and_the_same_seed_repeats_it() {
    let results: Vec<f64> = (0..200)
        .map(|deal| duel(&MATCH, deal, &shoves, &calls_good_cards))
        .collect();
    // Some deals the caller folds twice, and some it wins or loses a good share of a stack.
    assert!(results.contains(&0.75));
    assert!(results.iter().any(|result| result.abs() >= 5.0));
    let again: Vec<f64> = (0..200)
        .map(|deal| duel(&MATCH, deal, &shoves, &calls_good_cards))
        .collect();
    assert_eq!(results, again);
    let other = Match { seed: 10, ..MATCH };
    assert_ne!(
        results,
        (0..200)
            .map(|deal| duel(&other, deal, &shoves, &calls_good_cards))
            .collect::<Vec<f64>>()
    );
}

#[test]
fn the_verdict_is_the_mean_in_mbb_per_100_with_its_95_percent_interval() {
    // Two deals at 0 and 2 big blinds a hand: mean 1, standard error 1.
    let verdict = verdict(&[0.0, 2.0]);
    assert_eq!(verdict.deals, 2);
    assert!((verdict.mbb_per_100 - 100_000.0).abs() < 1e-6);
    assert!((verdict.ci95 - 196_000.0).abs() < 100.0, "{verdict:?}");
    assert!(!verdict.above_zero());
    assert!(super_verdict().above_zero());
}

fn super_verdict() -> svanbot_learn::paired::Verdict {
    verdict(&[1.0, 1.1, 0.9, 1.0])
}
