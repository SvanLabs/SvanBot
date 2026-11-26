//! M3–M5 through the pricer (V11-101): Grinder and Gambler answer spots their own way, and every
//! other style prices exactly as before.

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Legal, RaiseRange, Street};
use svanbot_core::spot::{Act, ActKind, Spot};
use svanbot_strategy::params::Params;
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::style::{LOOSE, STANDARD, TIGHT};

fn budget() -> Budget {
    Budget {
        samples: 400,
        seed: 7,
    }
}

fn preflop_spot(hole: &str) -> Spot {
    Spot {
        hole: hole.parse().expect("a hole"),
        board: CardSet::EMPTY,
        pot: 90,
        callable_pot: None,
        legal: Legal {
            call: Some(60),
            raise: Some(RaiseRange {
                min: 120,
                max: 2_000,
            }),
        },
        opponents: 1,
        stack: 1_900,
        committed: 10,
        big_blind: 20,
        in_position: true,
        players_behind: Some(1),
        lines: vec![vec![Act {
            street: Street::Preflop,
            board: CardSet::EMPTY,
            facing: false,
            kind: ActKind::Bet { added: 60, pot: 30 },
        }]],
        seats: vec![2],
    }
}

fn flop_spot(board: &str) -> Spot {
    Spot {
        hole: "3c4s".parse().expect("a hole"),
        board: board.parse().expect("a board"),
        pot: 100,
        callable_pot: None,
        legal: Legal {
            call: None,
            raise: Some(RaiseRange {
                min: 20,
                max: 2_000,
            }),
        },
        opponents: 1,
        stack: 1_900,
        committed: 0,
        big_blind: 20,
        in_position: true,
        players_behind: Some(1),
        lines: vec![vec![Act {
            street: Street::Flop,
            board: board.parse().expect("a board"),
            facing: false,
            kind: ActKind::Check,
        }]],
        seats: vec![2],
    }
}

#[test]
fn gambler_three_bets_a_bluff_the_reference_folds() {
    let spot = preflop_spot("7s2c");
    let bluffing = Params {
        three_bet_bluff: 0.2,
        ..Params::DEFAULT
    };
    assert_eq!(
        pricer::price_lines_styled(&spot, &[], budget(), &LOOSE, &bluffing).action,
        Action::RaiseTo(210),
        "the worst class is a polarised three-bet for Gambler"
    );
    assert_eq!(
        pricer::price_lines_at(&spot, &[], budget(), 1.0).action,
        Action::Fold,
        "the reference folds the same hand"
    );
}

#[test]
fn grinder_bets_a_dry_board_for_its_own_size() {
    let spot = flop_spot("Qh7c2d");
    let betting = Params {
        cbet_dry: 1.0,
        ..Params::DEFAULT
    };
    assert_eq!(
        pricer::price_lines_styled(&spot, &[], budget(), &TIGHT, &betting).action,
        Action::RaiseTo(33),
        "Grinder bets a third of the pot on a dry board"
    );
}

#[test]
fn every_neutral_style_prices_exactly_as_the_reference() {
    for spot in [
        preflop_spot("AsAc"),
        preflop_spot("7s2c"),
        flop_spot("Qh7c2d"),
    ] {
        let styled = pricer::price_lines_styled(&spot, &[], budget(), &STANDARD, &Params::DEFAULT);
        let reference = pricer::price_lines_at(&spot, &[], budget(), 1.0);
        assert_eq!(styled, reference);
    }
}
