//! v10.0.2 at the sim's table: the seat reads the table as v10's engine showed it a hand, and
//! every action it takes there is legal.

use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Blinds};
use svanbot_sim::table::{deal, play_seen};
use svanbot_sim::v10::cards::Card;
use svanbot_sim::v10::seat::{V10, situation};
use svanbot_sim::v10::situation::{ActionKind, ActionRecord, PlayerInfo, Street};

const BLINDS: Blinds = Blinds { small: 10, big: 20 };

#[test]
fn the_seat_reads_the_table_as_v10_s_engine_showed_it() {
    // Three seats, the button on 0: 0 raises to 60, 1 calls from the small blind, and the big
    // blind is to act.
    let cards = deal(3, &mut Rng::seeded(11));
    let mut read = None;
    play_seen(
        &cards,
        &[2000, 2000, 2000],
        0,
        BLINDS,
        1,
        |seat, spot, seen| {
            if seat == 2 && read.is_none() {
                read = Some(situation(seat, spot, seen));
            }
            match seat {
                0 => Action::RaiseTo(60),
                1 => Action::Call,
                _ => Action::Fold,
            }
        },
    );
    let sit = read.expect("the big blind acted");
    let v10 = |card: svanbot_core::card::Card| Card::parse(&card.to_string()).unwrap();
    let mut hole = cards.holes[2].iter().map(v10).collect::<Vec<_>>();
    let mut read_hole = sit.hole.to_vec();
    hole.sort_by_key(|c| c.to_string());
    read_hole.sort_by_key(|c| c.to_string());
    assert_eq!(read_hole, hole);
    assert_eq!(
        (sit.hero_seat, sit.street, sit.button, sit.bb, sit.pot),
        (2, Street::Preflop, 0, 20, 140)
    );
    assert!(sit.board.is_empty());
    assert_eq!(
        (sit.call_amount, sit.current_bet_to, sit.can_check),
        (40, Some(60), false)
    );
    assert_eq!(
        (sit.min_raise_to, sit.max_raise_to),
        (Some(100), Some(2000))
    );
    let player = |seat, stack, bet| PlayerInfo {
        seat,
        stack,
        bet,
        folded: false,
    };
    assert_eq!(
        sit.players,
        [
            player(0, 1940, 60),
            player(1, 1940, 60),
            player(2, 1980, 20)
        ]
    );
    let raise = ActionRecord {
        seat: 0,
        street: Street::Preflop,
        kind: ActionKind::Raise,
        to: 60,
        pot_before: 30,
        to_call_before: 20,
        bet_before: 0,
        full_raise: false,
    };
    let call = ActionRecord {
        seat: 1,
        kind: ActionKind::Call,
        pot_before: 90,
        to_call_before: 50,
        bet_before: 10,
        ..raise
    };
    assert_eq!(sit.history, [raise, call]);
}

#[test]
fn every_action_v10_takes_at_the_table_is_legal() {
    // Six seats of v10.0.2, a few deals: the table plays each decision as asked only if it is
    // legal, so any action it had to correct is caught here.
    let v10 = V10::default();
    for seed in 0..3 {
        let cards = deal(6, &mut Rng::seeded(seed));
        let mut asked = 0;
        play_seen(&cards, &[2000; 6], 0, BLINDS, seed, |seat, spot, seen| {
            asked += 1;
            let action = v10.decide(seat, spot, seen, seed * 100 + asked);
            let legal = spot.legal;
            let ok = match action {
                Action::Fold => true,
                Action::Check => legal.call.is_none(),
                Action::Call => legal.call.is_some(),
                Action::RaiseTo(to) => legal.raise.is_some_and(|r| (r.min..=r.max).contains(&to)),
            };
            assert!(ok, "seed {seed}: {action:?} at {legal:?}");
            action
        });
        assert!(asked > 0);
    }
}
