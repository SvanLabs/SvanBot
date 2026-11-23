//! The reads' sim: a warm-up builds the challenger's read of a sparring opponent with the live
//! code, and a paired comparison measures what playing by it wins over playing without it.

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::paired::Match;
use svanbot_learn::read_sim::{OPPONENT, reads_on_off, warm_up};
use svanbot_sim::sparring::Archetype;
use svanbot_strategy::read::Class;

const MATCH: Match = Match {
    blinds: Blinds { small: 10, big: 20 },
    stack: 2000,
    seed: 9,
};

/// Bets the pot when it can open, and calls otherwise: it puts the opponent to a decision.
fn bettor(spot: &Spot, _: u64) -> Action {
    match (spot.legal.call, spot.legal.raise) {
        (None, Some(range)) => Action::RaiseTo((spot.pot).clamp(range.min, range.max)),
        (Some(_), _) => Action::Call,
        (None, None) => Action::Check,
    }
}

#[test]
fn a_warm_up_reads_the_calling_station_as_one_fixed_opponent() {
    let reads = warm_up(&MATCH, Archetype::Station, 0..40, &bettor);
    let names: Vec<&str> = reads.opponents().map(|opponent| opponent.name()).collect();
    assert_eq!(names, [OPPONENT]);
    let villain = reads.get(OPPONENT).unwrap();
    // Both hands of each deal: 80 hands, and the station never folds to a bet.
    assert_eq!(villain.hands(), 80);
    assert_eq!(villain.class(), Class::Fixed);
    assert!(villain.earned() > 0.5, "{}", villain.earned());
    assert_eq!(warm_up(&MATCH, Archetype::Station, 0..40, &bettor), reads);
}

#[test]
fn the_same_decider_on_both_sides_wins_nothing_from_reads() {
    let results = reads_on_off(&MATCH, Archetype::Nit, 100..110, &bettor, &bettor);
    assert_eq!(results.len(), 10);
    assert!(
        results.iter().all(|result| result.abs() < 1e-9),
        "{results:?}"
    );
}
