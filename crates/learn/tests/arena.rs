//! The Arena: a policy's paired results against each sparring archetype, as a scoreboard.

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::arena::{BOUT, Row, bout, ring_bout, scoreboard, v10_duel, v10_ring};
use svanbot_learn::paired::{Match, Verdict};
use svanbot_sim::sparring::Archetype;

const MATCH: Match = Match {
    blinds: Blinds { small: 10, big: 20 },
    stack: 2000,
    seed: 4,
};

fn folds(_: &Spot, _: u64) -> Action {
    Action::Fold
}

fn check_or_call(spot: &Spot, _: u64) -> Action {
    match spot.legal.call {
        Some(_) => Action::Call,
        None => Action::Check,
    }
}

#[test]
fn a_seat_that_always_folds_breaks_even_with_the_nit() {
    // The nit folds its small blind and we fold ours: 10 each way on every deal.
    let results = bout(&MATCH, Archetype::Nit, 0, &folds);
    assert_eq!(results.len() as u64, BOUT);
    assert!(
        results.iter().all(|result| result.abs() < 1e-9),
        "{results:?}"
    );
}

#[test]
fn a_bout_repeats_for_its_seed_and_differs_by_its_number() {
    let first = bout(&MATCH, Archetype::Learning, 3, &check_or_call);
    assert_eq!(first, bout(&MATCH, Archetype::Learning, 3, &check_or_call));
    assert_ne!(first, bout(&MATCH, Archetype::Learning, 4, &check_or_call));
}

#[test]
fn a_seat_that_always_folds_breaks_even_with_five_nits() {
    // Round a full table the challenger posts each blind once, and the nits never open: every
    // deal's blinds go back and forth to nothing.
    let results = ring_bout(&MATCH, Archetype::Nit, 0, &folds);
    assert_eq!(results.len() as u64, BOUT);
    assert!(
        results.iter().all(|result| result.abs() < 1e-9),
        "{results:?}"
    );
}

#[test]
fn a_six_max_bout_repeats_for_its_seed_and_differs_by_its_number() {
    let first = ring_bout(&MATCH, Archetype::Learning, 1, &check_or_call);
    assert_eq!(
        first,
        ring_bout(&MATCH, Archetype::Learning, 1, &check_or_call)
    );
    assert_ne!(
        first,
        ring_bout(&MATCH, Archetype::Learning, 2, &check_or_call)
    );
}

#[test]
fn a_seat_that_always_folds_against_v10_loses_its_blinds_or_wins_the_small_one() {
    // Our button folds its small blind. In the big blind we fold to any raise or limp, and
    // win the small blind when v10 folds it: 0 or -30 chips over the deal, of 40 a hand.
    for deal in 0..3 {
        let won = v10_duel(&MATCH, deal, &folds);
        assert!(
            won == 0.0 || (won + 0.75).abs() < 1e-9,
            "deal {deal}: {won}"
        );
        assert_eq!(won, v10_duel(&MATCH, deal, &folds));
    }
}

#[test]
fn a_seat_that_always_folds_at_a_table_of_v10_gives_up_chips() {
    let won = v10_ring(&MATCH, 0, &folds);
    assert!(won <= 0.0, "{won}");
    assert_eq!(won, v10_ring(&MATCH, 0, &folds));
}

#[test]
fn the_scoreboard_lists_each_opponent_with_its_win_rate_and_interval() {
    let rows = [
        Row {
            opponent: "nit".into(),
            verdict: Verdict {
                deals: 400,
                mbb_per_100: 12_345.6,
                ci95: 2_000.4,
            },
        },
        Row {
            opponent: "maniac".into(),
            verdict: Verdict {
                deals: 400,
                mbb_per_100: -500.0,
                ci95: 9_000.0,
            },
        },
    ];
    let board = scoreboard(&rows);
    assert!(board.contains("nit") && board.contains("+12346") && board.contains("±2000"));
    assert!(board.contains("maniac") && board.contains("-500") && board.contains("±9000"));
}
