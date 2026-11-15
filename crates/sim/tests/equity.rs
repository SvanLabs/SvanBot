//! Equity: each player's share of the pot over the ways the board can run out.

use svanbot_core::card::CardSet;
use svanbot_sim::equity::{exact, sampled};

fn cards(text: &str) -> CardSet {
    text.parse().unwrap()
}

#[test]
fn on_the_river_the_best_hand_has_it_all() {
    let shares = exact(&[cards("AhAd"), cards("KcKd")], cards("2c7d9hJsQc"));
    assert_eq!(shares, [1.0, 0.0]);
    let split = exact(&[cards("AhKd"), cards("AcKh")], cards("2c7d9hJsQc"));
    assert_eq!(split, [0.5, 0.5]);
}

#[test]
fn on_the_turn_equity_is_the_outs_over_the_cards_left() {
    // AhKh against 2c2d on Qh7h3s9c: any of 9 hearts, 3 aces or 3 kings wins for AhKh (the 2h
    // makes a flush over trips), and the other 29 of the 44 unseen cards lose.
    let shares = exact(&[cards("AhKh"), cards("2c2d")], cards("Qh7h3s9c"));
    assert_eq!(shares, [15.0 / 44.0, 29.0 / 44.0]);
}

#[test]
fn relabelling_the_suits_changes_no_equity() {
    let before = exact(
        &[cards("AhKh"), cards("Jc9c"), cards("5s5d")],
        cards("Th8c2h"),
    );
    let after = exact(
        &[cards("AsKs"), cards("Jd9d"), cards("5h5c")],
        cards("Ts8d2s"),
    );
    assert_eq!(before, after);
    assert!((before.iter().sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn sampling_aces_against_kings_gives_about_82_percent() {
    let kings = [(cards("KcKd"), 1.0)];
    let equity = sampled(cards("AhAs"), &[&kings], CardSet::EMPTY, 64_000, 1);
    // The published preflop figure is about 82%. 64,000 deals give a standard deviation of 0.15%.
    assert!((0.80..0.84).contains(&equity), "{equity}");
}

#[test]
fn a_seed_gives_the_same_bits_every_time_and_another_seed_does_not() {
    let range = [
        (cards("KcKd"), 2.0),
        (cards("QcQd"), 1.0),
        (cards("Jc9c"), 1.0),
    ];
    let run = |seed| {
        sampled(
            cards("AhKh"),
            &[&range, &range],
            cards("Th8c2h"),
            6_400,
            seed,
        )
    };
    assert_eq!(run(7).to_bits(), run(7).to_bits());
    assert_ne!(run(7).to_bits(), run(8).to_bits());
}

#[test]
fn sampling_agrees_with_enumeration() {
    let hero = cards("AhKh");
    let villain = [(cards("2c2d"), 1.0)];
    let equity = sampled(hero, &[&villain], cards("Qh7h3s"), 64_000, 3);
    let truth = exact(&[hero, cards("2c2d")], cards("Qh7h3s"))[0];
    assert!((equity - truth).abs() < 0.01, "{equity} vs {truth}");
}

#[test]
fn a_range_with_nothing_in_it_deals_nothing() {
    let empty: [(CardSet, f64); 0] = [];
    assert_eq!(
        sampled(cards("AhAs"), &[&empty], CardSet::EMPTY, 640, 1),
        0.0
    );
}
