//! The Arena: a policy's paired results against each sparring archetype, as a scoreboard.

use std::cell::RefCell;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::arena::{
    BOUT, Row, bout, bout_against, ring_bout, scoreboard, v10_duel, v10_ring,
};
use svanbot_learn::paired::{Match, Verdict};
use svanbot_sim::sparring::{Archetype, Memory, Sparring};
use svanbot_sim::table::deal;

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
fn an_arena_challenger_learns_between_deals_and_freezes_its_read_across_the_mirror() {
    use svanbot_learn::arena::BoutReader;
    use svanbot_learn::read_sim::OPPONENT;
    use svanbot_learn::reads::Reads;

    let counts = RefCell::new(Vec::new());
    let challenger = BoutReader::new(|spot: &Spot, seed, reads: &Reads| {
        counts
            .borrow_mut()
            .push(reads.get(OPPONENT).map_or(0, |r| r.hands()));
        check_or_call(spot, seed)
    });
    bout(&MATCH, Archetype::Station, 0, &challenger);
    let counts = counts.into_inner();
    assert_eq!(counts.first(), Some(&0));
    assert_eq!(counts.last(), Some(&98));
    assert!(counts.iter().all(|n| n % 2 == 0));
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
fn a_ring_bout_keeps_each_opponents_read_across_deals() {
    use svanbot_learn::arena::{BoutReader, opponent_name};
    use svanbot_learn::reads::Reads;
    use svanbot_learn::ring::SIX_MAX;

    let last = RefCell::new(Vec::new());
    let challenger = BoutReader::new(|spot: &Spot, seed, reads: &Reads| {
        *last.borrow_mut() = reads
            .opponents()
            .map(|r| (r.name().to_owned(), r.hands()))
            .collect();
        folds(spot, seed)
    });
    ring_bout(&MATCH, Archetype::Nit, 0, &challenger);
    // Before the final deal, each seat has been an opponent in five rotations of 49 deals.
    let expected: Vec<_> = (0..SIX_MAX)
        .map(|seat| (opponent_name(SIX_MAX, seat), 245))
        .collect();
    assert_eq!(last.into_inner(), expected);
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

#[test]
fn the_learning_opponent_learns_across_a_bout_and_both_hands_of_a_deal_meet_it_the_same() {
    // What the opponent remembers when the challenger first acts in a hand, with the
    // challenger's cards, in play order. Preflop it has seen nothing of this hand's challenger
    // yet, so this is what it carried into the hand. A hand the opponent folds before we act
    // leaves no entry.
    let opponent = Sparring::new(Archetype::Learning);
    let carried = RefCell::new(Vec::new());
    bout_against(&MATCH, &opponent, 0, &|spot: &Spot, seed| {
        let mut carried = carried.borrow_mut();
        if carried.last().is_none_or(|&(hole, _)| hole != spot.hole) {
            carried.push((spot.hole, opponent.memory()));
        }
        check_or_call(spot, seed)
    });
    let mut carried = carried.into_inner().into_iter().peekable();
    // The deal's two hands give the challenger each seat's cards in turn.
    let hands: Vec<_> = (0..BOUT)
        .map(|number| {
            let cards = deal(2, &mut Rng::stream(MATCH.seed, number));
            [0, 1].map(|seat| {
                carried
                    .next_if(|&(hole, _)| hole == cards.holes[seat])
                    .map(|(_, memory)| memory)
            })
        })
        .collect();
    assert!(carried.next().is_none(), "an entry matched no hand");
    let mut compared = 0;
    for (number, pair) in hands.iter().enumerate() {
        if let [Some(first), Some(second)] = pair {
            assert_eq!(
                first, second,
                "deal {number}: its hands met different opponents"
            );
            compared += 1;
        }
    }
    assert!(compared > BOUT / 2, "only {compared} deals had both hands");
    let seen = |pair: &[Option<Memory>; 2]| pair.iter().flatten().next().copied();
    let first = hands.iter().find_map(seen).unwrap();
    let last = hands.iter().rev().find_map(seen).unwrap();
    assert!(
        last.acts > first.acts + BOUT as u32 / 2,
        "it forgot between deals: {first:?} then {last:?}"
    );
}

#[test]
fn two_builds_are_compared_bout_by_bout_over_the_same_deals() {
    use svanbot_learn::arena::compare;

    // Bout sums are big blinds a hand summed over a bout's 50 deals: 5 and 10 more a bout is
    // 0.1 and 0.2 bb a hand, a mean of 15,000 mbb/100 with a standard error of 0.05 bb a hand.
    let found = compare(&[10.0, 20.0], &[15.0, 30.0]).unwrap();
    assert_eq!(found.deals, 100);
    assert!((found.mbb_per_100 - 15_000.0).abs() < 1e-6, "{found:?}");
    assert!((found.ci95 - 9_800.0).abs() < 1e-6, "{found:?}");

    let same = compare(&[3.0, -7.0, 1.5], &[3.0, -7.0, 1.5]).unwrap();
    assert_eq!((same.mbb_per_100, same.ci95, same.deals), (0.0, 0.0, 150));
}

#[test]
fn runs_of_different_lengths_are_not_compared() {
    use svanbot_learn::arena::compare;

    assert_eq!(compare(&[1.0, 2.0], &[1.0]), None);
    assert_eq!(compare(&[], &[]), None);
}
