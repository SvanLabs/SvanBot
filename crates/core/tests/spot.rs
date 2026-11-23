//! The spot's opponents: a line and the seat it came from (V11-24, #1103).

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Legal, Street};
use svanbot_core::spot::{Act, ActKind, Spot};

fn bare() -> Spot {
    Spot {
        hole: CardSet::EMPTY,
        board: CardSet::EMPTY,
        pot: 0,
        legal: Legal {
            call: None,
            raise: None,
        },
        opponents: 0,
        stack: 0,
        callable_pot: None,
        committed: 0,
        big_blind: 0,
        in_position: false,
        seats: Vec::new(),
        lines: Vec::new(),
    }
}

#[test]
fn an_opponents_line_knows_its_seat() {
    // A range in a trace has to name its opponent by something a reader can find at the table,
    // which is the seat. The seat rides with the line, one entry each, so a caller that reorders
    // or filters the lines still knows which opponent each one belongs to.
    let called = Act {
        street: Street::Preflop,
        board: CardSet::EMPTY,
        facing: false,
        kind: ActKind::Call,
    };
    let spot = Spot {
        opponents: 2,
        seats: vec![3, 5],
        lines: vec![vec![called], vec![]],
        ..bare()
    };
    assert_eq!(spot.seats, vec![3, 5]);
    assert_eq!(spot.lines.len(), spot.seats.len());
}

#[test]
fn every_line_has_a_seat_or_the_spot_names_none_at_all() {
    // The invariant the two fields share: seats is empty (a caller that did not know them), or it
    // has one entry per line. Never a partial list, which would pair some lines with the wrong
    // seat — a wrong label that reads as a right one.
    let spot = Spot {
        opponents: 2,
        seats: vec![4, 1],
        lines: vec![vec![], vec![]],
        ..bare()
    };
    assert_eq!(spot.seats.len(), spot.lines.len());

    let unknown = Spot {
        opponents: 2,
        lines: vec![vec![], vec![]],
        ..bare()
    };
    assert!(unknown.seats.is_empty());
}

#[test]
fn a_spot_with_no_opponents_has_no_seats() {
    let spot = bare();
    assert_eq!(spot.opponents, 0);
    assert!(spot.seats.is_empty());
    assert_eq!(
        spot.legal,
        Legal {
            call: None,
            raise: None
        }
    );
}
