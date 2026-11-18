//! The Arena: a policy's paired results against each sparring archetype, as a scoreboard.

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::arena::{BOUT, Row, bout, scoreboard};
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
