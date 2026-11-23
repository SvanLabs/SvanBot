//! The sparring archetypes: scripted opponents that stand in for kinds of real ones.

use svanbot_core::rules::{Action, Legal, RaiseRange, Street};
use svanbot_core::spot::{Act, ActKind, Spot};
use svanbot_sim::sparring::{Archetype, PERIOD, Sparring};

fn spot(hole: &str, board: &str, pot: u32, call: Option<u32>) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal: Legal {
            call,
            raise: Some(RaiseRange {
                min: call.map_or(20, |call| 2 * call),
                max: 2000,
            }),
        },
        opponents: 1,
        stack: 2000,
        committed: 0,
        big_blind: 20,
        in_position: true,
        lines: vec![vec![]],
        // One seat per line, as the invariant says: this spot has no trace, but a fixture that
        // breaks the invariant is a trap for the next reader.
        seats: vec![0],
    }
}

const BOARD: &str = "Ks9d4c7h2s";

#[test]
fn the_calling_station_calls_any_bet_and_bets_only_its_best_hands() {
    let station = Sparring::new(Archetype::Station);
    for seed in 0..20 {
        // Third pair calls a pot-sized bet, and checks when checked to.
        let facing = spot("7c3d", BOARD, 400, Some(200));
        assert_eq!(station.decide(&facing, seed, 0), Action::Call);
        assert_eq!(
            station.decide(&spot("7c3d", BOARD, 200, None), seed, 0),
            Action::Check
        );
        // A set of kings bets when checked to.
        let set = station.decide(&spot("KcKd", BOARD, 200, None), seed, 0);
        assert!(matches!(set, Action::RaiseTo(_)), "{set:?}");
    }
}

#[test]
fn the_nit_folds_all_but_strong_hands_to_a_bet_and_raises_those() {
    let nit = Sparring::new(Archetype::Nit);
    for seed in 0..20 {
        // Second pair folds to a bet; a set raises it.
        assert_eq!(
            nit.decide(&spot("9c3d", BOARD, 400, Some(200)), seed, 0),
            Action::Fold
        );
        let set = nit.decide(&spot("KcKd", BOARD, 400, Some(200)), seed, 0);
        assert!(matches!(set, Action::RaiseTo(_)), "{set:?}");
        assert_eq!(
            nit.decide(&spot("9c3d", BOARD, 200, None), seed, 0),
            Action::Check
        );
    }
}

#[test]
fn the_maniac_bets_and_raises_most_of_the_time_with_anything() {
    let maniac = Sparring::new(Archetype::Maniac);
    let raises = (0..200)
        .filter(|&seed| {
            let action = maniac.decide(&spot("7c3d", BOARD, 400, Some(200)), seed, 0);
            assert_ne!(action, Action::Fold);
            matches!(action, Action::RaiseTo(_))
        })
        .count();
    assert!((100..=160).contains(&raises), "{raises} of 200");
}

#[test]
fn the_switching_archetype_is_a_nit_then_a_maniac_by_the_hand() {
    let switching = Sparring::new(Archetype::Switching);
    let facing = spot("9c3d", BOARD, 400, Some(200));
    for seed in 0..20 {
        assert_eq!(switching.decide(&facing, seed, 10), Action::Fold);
        assert_ne!(switching.decide(&facing, seed, PERIOD + 10), Action::Fold);
        assert_eq!(
            switching.decide(&facing, seed, 2 * PERIOD + 10),
            Action::Fold
        );
    }
}

#[test]
fn a_short_arena_run_still_plays_both_halves_of_the_switching_archetype() {
    // The Arena's smallest run is two bouts, 100 deals (`BOUT`: 50). A period of 200 would leave
    // that whole run in the nit half, so the switching row and the nit row would be one test.
    let switching = Sparring::new(Archetype::Switching);
    let facing = spot("9c3d", BOARD, 400, Some(200));
    let nit = (0..100)
        .filter(|hand| switching.decide(&facing, 1, *hand) == Action::Fold)
        .count();
    assert!(
        (20..=80).contains(&nit),
        "the maniac half must show in 100 deals, not {nit} nit folds of 100"
    );
}

#[test]
fn the_learning_archetype_calls_lighter_once_it_has_seen_us_bet_and_bet() {
    let bet = Act {
        street: Street::River,
        board: BOARD.parse().unwrap(),
        facing: false,
        kind: ActKind::Bet {
            added: 200,
            pot: 200,
        },
    };
    // Ace high facing a pot-sized bet: a fold against a player it knows nothing of.
    let mut facing = spot("Ac3d", BOARD, 400, Some(200));
    facing.lines = vec![vec![bet]];
    let learning = Sparring::new(Archetype::Learning);
    assert_eq!(learning.decide(&facing, 1, 0), Action::Fold);
    // After a run of our bets it takes us for a bluffer, and calls.
    for hand in 1..40 {
        learning.decide(&facing, 1, hand);
    }
    assert_eq!(learning.decide(&facing, 1, 40), Action::Call);
    // A fresh one has not learned it.
    assert_eq!(
        Sparring::new(Archetype::Learning).decide(&facing, 1, 41),
        Action::Fold
    );
}

#[test]
fn at_a_full_table_the_learning_archetype_learns_from_the_seat_it_watches() {
    let act = |kind| Act {
        street: Street::River,
        board: BOARD.parse().unwrap(),
        facing: false,
        kind,
    };
    let bet = act(ActKind::Bet {
        added: 200,
        pot: 200,
    });
    // Seat 2 bets every hand and seat 4, the one it watches, only checks: seat 2's bets say
    // nothing of us, so it still folds ace high.
    let mut facing = spot("Ac3d", BOARD, 400, Some(200));
    facing.lines = vec![vec![bet], vec![act(ActKind::Check)]];
    facing.seats = vec![2, 4];
    let learning = Sparring::new(Archetype::Learning);
    learning.watch(4);
    for hand in 0..40 {
        learning.decide(&facing, 1, hand);
    }
    assert_eq!(learning.decide(&facing, 1, 40), Action::Fold);
    // Watching seat 2 instead, the same bets make it call.
    let watching = Sparring::new(Archetype::Learning);
    watching.watch(2);
    for hand in 0..40 {
        watching.decide(&facing, 1, hand);
    }
    assert_eq!(watching.decide(&facing, 1, 40), Action::Call);
}
