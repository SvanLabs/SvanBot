//! The preflop tables (V11-39): each of the 169 starting hands' equity against one to five random
//! hands, shipped in the binary, and the hands each seat opens with. The expected equities are
//! the published all-in figures, not numbers derived from the code under test.

use svanbot_core::card::CardSet;
use svanbot_strategy::preflop::{CLASSES, Seat, class_of, equity, name, opens};

fn class(hole: &str) -> usize {
    class_of(hole.parse::<CardSet>().unwrap())
}

#[test]
fn the_table_matches_the_published_equities() {
    let near = |hole: &str, opponents: usize, published: f64, within: f64| {
        let found = equity(class(hole), opponents);
        assert!(
            (found - published).abs() <= within,
            "{hole} against {opponents}: {found} is not within {within} of {published}"
        );
    };
    near("AhAs", 1, 0.852, 0.004);
    near("KhKs", 1, 0.824, 0.004);
    near("7h2s", 1, 0.346, 0.004);
    near("AhKh", 1, 0.670, 0.004);
    near("AhAs", 5, 0.492, 0.010);
}

#[test]
fn aces_lead_at_every_table_size_and_equity_falls_as_opponents_are_added() {
    let aces = class("AhAs");
    for opponents in 1..=5 {
        for other in 0..CLASSES {
            assert!(
                equity(other, opponents) <= equity(aces, opponents),
                "{} beats aces against {opponents}",
                name(other)
            );
            if opponents > 1 {
                assert!(
                    equity(other, opponents) < equity(other, opponents - 1),
                    "{}",
                    name(other)
                );
            }
        }
    }
}

#[test]
fn every_pair_of_hole_cards_has_one_of_169_classes_and_suits_do_not_matter() {
    assert_eq!(CLASSES, 169);
    assert_eq!(class("AhKh"), class("AsKs"));
    assert_ne!(class("AhKh"), class("AhKs"));
    assert_eq!(class("7h2s"), class("2c7d"));
    assert_eq!(name(class("AhAs")), "AA");
    assert_eq!(name(class("AhKh")), "AKs");
    assert_eq!(name(class("7h2s")), "72o");
}

#[test]
fn each_seat_opens_its_best_hands_and_the_later_seats_open_more() {
    let utg = Seat {
        behind: 5,
        in_position: false,
    };
    let button = Seat {
        behind: 2,
        in_position: true,
    };
    let heads_up_button = Seat {
        behind: 1,
        in_position: true,
    };
    assert!(opens(class("AhKs"), utg));
    assert!(opens(class("QhQs"), utg));
    assert!(!opens(class("7h2s"), utg));
    assert!(!opens(class("Kh9s"), utg));
    assert!(opens(class("Kh9s"), button));
    assert!(opens(class("Kh9s"), heads_up_button));
    assert!(opens(class("8h5s"), heads_up_button));
    let count = |seat: Seat| (0..CLASSES).filter(|&c| opens(c, seat)).count();
    assert!(
        count(utg) < count(button),
        "UTG {} BTN {}",
        count(utg),
        count(button)
    );
    assert!(count(button) < count(heads_up_button));
}
