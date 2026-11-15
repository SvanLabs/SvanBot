//! The table: a deal from a seed, and one hand played out through a decision per seat.

use std::collections::BTreeSet;

use svanbot_core::card::Card;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Blinds, Street};
use svanbot_core::spot::{ActKind, Spot};
use svanbot_sim::table::{Deal, deal, play, play_corrected};

const BLINDS: Blinds = Blinds { small: 10, big: 20 };

fn dealt(holes: &[&str], board: &str) -> Deal {
    let board: Vec<Card> = board
        .as_bytes()
        .chunks(2)
        .map(|card| std::str::from_utf8(card).unwrap().parse().unwrap())
        .collect();
    Deal {
        holes: holes.iter().map(|hole| hole.parse().unwrap()).collect(),
        board: board.try_into().unwrap(),
    }
}

fn check_or_call(spot: &Spot) -> Action {
    match spot.legal.call {
        Some(_) => Action::Call,
        None => Action::Check,
    }
}

#[test]
fn a_seed_deals_the_same_distinct_cards_every_time() {
    let first = deal(6, &mut Rng::seeded(7));
    assert_eq!(first, deal(6, &mut Rng::seeded(7)));
    assert_ne!(first, deal(6, &mut Rng::seeded(8)));
    let mut seen = BTreeSet::new();
    for hole in &first.holes {
        assert_eq!(hole.len(), 2);
        seen.extend(hole.iter().map(|card| card.to_string()));
    }
    seen.extend(first.board.iter().map(|card| card.to_string()));
    assert_eq!(seen.len(), 6 * 2 + 5);
}

#[test]
fn a_fold_before_the_flop_gives_the_big_blind_the_small_blind() {
    // Heads up the button posts the small blind and acts first.
    let cards = dealt(&["7c2d", "AhAs"], "Ks9d4c7h2s");
    let won = play(&cards, &[2000, 2000], 0, BLINDS, |_, _| Action::Fold);
    assert_eq!(won, [-10, 10]);
}

#[test]
fn checked_down_the_better_hand_wins_the_blinds() {
    let cards = dealt(&["KcKd", "AhAs"], "Qs9d4c7h2s");
    let won = play(&cards, &[2000, 2000], 0, BLINDS, |_, spot| {
        check_or_call(spot)
    });
    assert_eq!(won, [-20, 20]);
}

#[test]
fn each_decision_sees_its_own_cards_the_board_so_far_and_the_other_seat_s_line() {
    let cards = dealt(&["KcKd", "AhAs"], "Qs9d4c7h2s");
    let mut spots = Vec::new();
    // The button raises to 60 before the flop and bets 50 on the flop; the big blind calls.
    play(&cards, &[2000, 2000], 0, BLINDS, |seat, spot| {
        spots.push((seat, spot.clone()));
        match (seat, spot.board.len(), spot.legal.call) {
            (0, 0, _) => Action::RaiseTo(60),
            (0, 3, None) => Action::RaiseTo(50),
            (_, _, Some(_)) => Action::Call,
            _ => Action::Check,
        }
    });
    // The big blind's flop call: it checked first, and the button bet 50 into 120.
    let (seat, spot) = spots
        .iter()
        .find(|(seat, spot)| *seat == 1 && spot.board.len() == 3 && spot.legal.call.is_some())
        .expect("the big blind faced the flop bet");
    assert_eq!(*seat, 1);
    assert_eq!(spot.hole, "AhAs".parse().unwrap());
    assert_eq!(spot.board, "Qs9d4c".parse().unwrap());
    assert_eq!(
        (spot.pot, spot.legal.call, spot.opponents, spot.stack),
        (170, Some(50), 1, 1940)
    );
    assert_eq!((spot.committed, spot.big_blind), (0, 20));
    let line = &spot.lines[0];
    assert_eq!(line.len(), 2);
    assert_eq!(
        (line[0].street, line[0].facing, line[0].kind),
        (Street::Preflop, true, ActKind::Bet { added: 50, pot: 30 })
    );
    assert_eq!(
        (line[1].street, line[1].board, line[1].facing, line[1].kind),
        (
            Street::Flop,
            "Qs9d4c".parse().unwrap(),
            false,
            ActKind::Bet {
                added: 50,
                pot: 120
            }
        )
    );
}

#[test]
fn no_chip_is_made_or_lost_whatever_the_seats_ask_for() {
    let mut rng = Rng::seeded(21);
    for seats in 2..=6 {
        for _ in 0..40 {
            let cards = deal(seats, &mut rng);
            let stacks: Vec<u32> = (0..seats).map(|_| 40 + rng.below(3000) as u32).collect();
            let button = rng.below(seats as u64) as usize;
            let mut draws = Rng::seeded(rng.next_u64());
            let won = play(&cards, &stacks, button, BLINDS, |_, spot| {
                match draws.below(5) {
                    0 => Action::Fold,
                    1 => Action::Check,
                    2 => Action::Call,
                    // Often not legal: the table plays the nearest action that is.
                    3 => Action::RaiseTo(spot.pot),
                    _ => Action::RaiseTo(spot.legal.raise.map_or(1, |range| range.max)),
                }
            });
            assert_eq!(won.iter().sum::<i64>(), 0, "{cards:?} {stacks:?}");
            for (net, stack) in won.iter().zip(&stacks) {
                assert!(*net >= -i64::from(*stack));
            }
        }
    }
}

fn shove(spot: &Spot) -> Action {
    match spot.legal.raise {
        Some(range) => Action::RaiseTo(range.max),
        None => check_or_call(spot),
    }
}

#[test]
fn an_all_in_before_the_river_pays_each_seat_its_share_of_the_run_outs() {
    // Checked to the turn, then all in: AhKh wins on 15 of the 44 rivers (the equity test counts
    // them), whatever river was dealt. Each seat has 2,000 in.
    let cards = dealt(&["AhKh", "2c2d"], "Qh7h3s9c5s");
    let all_in_on_the_turn = |_: usize, spot: &Spot| match spot.board.len() {
        4 => shove(spot),
        _ => check_or_call(spot),
    };
    assert_eq!(
        play(&cards, &[2000, 2000], 0, BLINDS, all_in_on_the_turn),
        [-2000, 2000]
    );
    let fair = play_corrected(&cards, &[2000, 2000], 0, BLINDS, 1, all_in_on_the_turn);
    let share = 15.0 / 44.0 * 4000.0 - 2000.0;
    assert!((fair[0] - share).abs() < 1e-6, "{fair:?}");
    assert!((fair[0] + fair[1]).abs() < 1e-6, "{fair:?}");
}

#[test]
fn an_all_in_before_the_flop_is_paid_close_to_its_equity_and_a_hand_bet_to_the_river_as_dealt() {
    // Aces against kings hold about 82% of the time.
    let cards = dealt(&["AhAs", "KcKd"], "Kh7h3s9c5s");
    let fair = play_corrected(&cards, &[2000, 2000], 0, BLINDS, 1, |_, spot| shove(spot));
    let share = 0.82 * 4000.0 - 2000.0;
    assert!((fair[0] - share).abs() < 150.0, "{fair:?}");
    assert!((fair[0] + fair[1]).abs() < 1e-6, "{fair:?}");
    // The same seed pays the same.
    assert_eq!(
        fair,
        play_corrected(&cards, &[2000, 2000], 0, BLINDS, 1, |_, spot| shove(spot))
    );
    let checked = play_corrected(&cards, &[2000, 2000], 0, BLINDS, 1, |_, spot| {
        check_or_call(spot)
    });
    assert_eq!(checked, [-20.0, 20.0]);
}
