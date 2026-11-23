//! The rules of a hand as the venue plays them (<https://docs.openpoker.ai/llms-full.txt>, "Game
//! Rules" and "Valid Actions"): 10/20 blinds, raise-to amounts, side pots, no rake.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::eval::{Strength, strength};
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Blinds, Hand, RaiseRange, Street};

const VENUE: Blinds = Blinds { small: 10, big: 20 };

fn hand(stacks: &[u64], button: usize) -> Hand {
    Hand::new(stacks, button, VENUE)
}

fn play(hand: &mut Hand, actions: &[Action]) {
    for &action in actions {
        let seat = hand.to_act().expect("someone is to act");
        hand.act(action)
            .unwrap_or_else(|e| panic!("seat {seat} could not {action:?}: {e}"));
    }
}

#[test]
fn the_blinds_sit_left_of_the_button_and_the_next_seat_acts_first() {
    let hand = hand(&[2000; 6], 0);
    assert_eq!(hand.committed(), [0, 10, 20, 0, 0, 0]);
    assert_eq!(hand.to_act(), Some(3));
    let legal = hand.legal().unwrap();
    assert_eq!(legal.call, Some(20));
    assert_eq!(legal.raise, Some(RaiseRange { min: 40, max: 2000 }));
}

#[test]
fn heads_up_the_button_posts_the_small_blind_and_acts_first_preflop_only() {
    let mut hand = hand(&[2000, 2000], 1);
    assert_eq!(hand.committed(), [20, 10]);
    assert_eq!(hand.to_act(), Some(1));
    play(&mut hand, &[Action::Call, Action::Check]);
    assert_eq!(hand.street(), Street::Flop);
    assert_eq!(hand.to_act(), Some(0));
}

#[test]
fn a_raise_names_the_total_and_the_next_raise_must_grow_by_as_much() {
    let mut hand = hand(&[2000; 6], 0);
    play(&mut hand, &[Action::RaiseTo(60)]);
    // The raise was 40 over the big blind, so the next must reach 100.
    assert_eq!(hand.legal().unwrap().call, Some(60));
    assert_eq!(
        hand.legal().unwrap().raise,
        Some(RaiseRange {
            min: 100,
            max: 2000
        })
    );
    assert!(hand.act(Action::RaiseTo(99)).is_err());
    assert!(hand.act(Action::Check).is_err());
}

#[test]
fn checking_round_every_street_reaches_the_showdown() {
    let mut hand = hand(&[2000; 3], 0);
    play(&mut hand, &[Action::Call, Action::Call, Action::Check]);
    for street in [Street::Flop, Street::Turn, Street::River] {
        assert_eq!(hand.street(), street);
        // Postflop the first seat left of the button acts first.
        assert_eq!(hand.to_act(), Some(1));
        play(&mut hand, &[Action::Check, Action::Check, Action::Check]);
    }
    assert_eq!(hand.to_act(), None);
    assert!(hand.is_showdown());
}

/// Four seats, button 0: UTG (seat 3) raises to 100, the button calls, and the small blind goes
/// all in for 150, 50 over the bet, which is short of the 80 a full raise adds.
fn after_a_short_all_in() -> Hand {
    let mut hand = hand(&[2000, 150, 200, 2000], 0);
    play(
        &mut hand,
        &[Action::RaiseTo(100), Action::Call, Action::RaiseTo(150)],
    );
    hand
}

#[test]
fn a_short_all_in_does_not_reopen_the_betting() {
    let mut hand = after_a_short_all_in();
    play(&mut hand, &[Action::Fold]);
    assert_eq!(hand.to_act(), Some(3));
    let legal = hand.legal().unwrap();
    assert_eq!(legal.call, Some(50));
    assert_eq!(legal.raise, None);
}

#[test]
fn short_all_ins_that_add_up_to_a_full_raise_reopen_it() {
    let mut hand = after_a_short_all_in();
    // The big blind's all-in to 200 takes the bet 100 past where seat 3 acted.
    play(&mut hand, &[Action::RaiseTo(200)]);
    assert_eq!(
        hand.legal().unwrap().raise,
        Some(RaiseRange {
            min: 280,
            max: 2000
        })
    );
}

#[test]
fn a_bet_nobody_calls_comes_back_with_the_blinds() {
    let mut hand = hand(&[2000; 3], 0);
    play(
        &mut hand,
        &[Action::RaiseTo(100), Action::Fold, Action::Fold],
    );
    assert_eq!(hand.to_act(), None);
    assert!(!hand.is_showdown());
    let payouts = hand.payouts(|_| unreachable!("no showdown"));
    assert_eq!(payouts, [130, 0, 0]);
}

#[test]
fn each_side_pot_goes_to_the_best_hand_that_reached_it() {
    let mut hand = hand(&[300, 100, 200], 0);
    play(
        &mut hand,
        &[Action::RaiseTo(300), Action::Call, Action::Call],
    );
    assert!(hand.is_showdown());
    assert_eq!(hand.street(), Street::River);
    // Seat 1 holds the best hand, seat 2 the next, seat 0 the worst.
    let strengths = [of("7c5d4h3s2c"), of("AsAdKhQsJc"), of("KcKdQhJsTc")];
    // Main pot 300 to seat 1; the side pot of 200 to seat 2; seat 0's last 100 comes back.
    assert_eq!(hand.payouts(|seat| strengths[seat]), [100, 300, 200]);
}

#[test]
fn an_odd_chip_goes_to_the_first_winner_left_of_the_button() {
    let mut hand = Hand::new(&[25, 25, 25], 0, Blinds { small: 5, big: 10 });
    play(
        &mut hand,
        &[Action::RaiseTo(25), Action::Call, Action::Call],
    );
    let strengths = [of("7c5d4h3s2c"), of("AsKdQhJsTc"), of("AcKhQdJcTd")];
    assert_eq!(hand.payouts(|seat| strengths[seat]), [0, 38, 37]);
}

fn of(cards: &str) -> Strength {
    strength(cards.parse::<CardSet>().unwrap())
}

#[test]
fn random_hands_keep_every_chip_and_accept_every_legal_action() {
    let mut rng = Rng::seeded(11);
    for _ in 0..3_000 {
        let seats = 2 + rng.below(5) as usize;
        let stacks: Vec<u64> = (0..seats).map(|_| 1 + rng.below(3_000)).collect();
        let mut hand = hand(&stacks, rng.below(seats as u64) as usize);
        let mut turns = 0;
        while let Some(legal) = hand.legal() {
            turns += 1;
            assert!(turns < 200, "the hand never ends");
            if let Some(range) = legal.raise
                && range.min > legal.call.unwrap_or(0) + 1
            {
                let below_min = Action::RaiseTo(range.min - 1);
                assert!(
                    hand.act(below_min).is_err(),
                    "{below_min:?} under {range:?}"
                );
            }
            let action = match (rng.below(4), legal.call, legal.raise) {
                (0, _, _) => Action::Fold,
                (1, _, Some(range)) => {
                    Action::RaiseTo(range.min + rng.below(range.max - range.min + 1))
                }
                (_, Some(_), _) => Action::Call,
                (_, None, _) => Action::Check,
            };
            hand.act(action).unwrap();
            let held: Vec<u64> = hand
                .stacks()
                .iter()
                .zip(hand.committed())
                .map(|(s, c)| s + c)
                .collect();
            assert_eq!(held, stacks, "chips came or went");
        }
        let strengths: Vec<Strength> = (0..seats)
            .map(|_| {
                let mut cards = CardSet::EMPTY;
                while cards.len() < 5 {
                    let rank = Rank::ALL[rng.below(13) as usize];
                    let suit = Suit::ALL[rng.below(4) as usize];
                    cards.insert(Card::new(rank, suit));
                }
                strength(cards)
            })
            .collect();
        let payouts = hand.payouts(|seat| strengths[seat]);
        assert_eq!(payouts.iter().sum::<u64>(), hand.pot());
    }
}

#[test]
fn a_board_names_its_street_and_every_street_names_itself() {
    let cards: Vec<Card> = ["Ah", "Kh", "Qh", "Jh", "Th"]
        .into_iter()
        .map(|text| text.parse().expect("a card"))
        .collect();
    let board = |n: usize| cards[..n].iter().copied().collect::<CardSet>();

    assert_eq!(Street::of(board(0)), Street::Preflop);
    assert_eq!(Street::of(board(3)), Street::Flop);
    assert_eq!(Street::of(board(4)), Street::Turn);
    assert_eq!(Street::of(board(5)), Street::River);
}

#[test]
fn every_street_reads_back_from_its_name_and_no_other() {
    for street in Street::ALL {
        assert_eq!(Street::named(street.name()), Some(street));
    }
    assert_eq!(Street::named("showdown"), None);
}

#[test]
fn chips_above_every_seat_still_in_come_back() {
    // Heads-up, the big blind is all in for 9, less than the small blind's 10. The small blind
    // folds to the full big blind, loses the 9 matched, and takes back the 1 nobody matched.
    let mut hand = hand(&[2000, 9], 0);
    play(&mut hand, &[Action::Fold]);
    assert_eq!(hand.payouts(|_| unreachable!("no showdown")), [1, 18]);
}
