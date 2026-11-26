//! A bot's own field: an action model fitted from its own store, with no identity in it at all,
//! and the name its Arena row is recorded under, apart from the release rows.

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Legal, RaiseRange, Street};
use svanbot_core::spot::Spot;
use svanbot_learn::field::{Field, is_row, row_name};
use svanbot_learn::fit::{Observed, Outcome};
use svanbot_learn::play::apart;
use svanbot_strategy::action::{Move, Seen, Situation};

fn observed(hand: u64, situation: Situation, mv: Move) -> Observed {
    Observed {
        hand,
        seen: vec![Seen {
            board: CardSet::EMPTY,
            situation,
            mv,
        }],
        dead: CardSet::EMPTY,
        outcome: Outcome::Unseen,
    }
}

fn preflop(facing: bool) -> Situation {
    Situation {
        street: Street::Preflop,
        facing,
    }
}

fn spot(hole: &str, call: Option<u64>) -> Spot {
    let committed = 20;
    let min = committed + call.unwrap_or(0) + 20;
    Spot {
        hole: hole.parse().unwrap(),
        board: CardSet::EMPTY,
        pot: 100,
        callable_pot: None,
        legal: Legal {
            call,
            raise: Some(RaiseRange { min, max: 1_000 }),
        },
        opponents: 1,
        stack: 1_000,
        committed,
        big_blind: 20,
        in_position: true,
        players_behind: None,
        lines: Vec::new(),
        seats: Vec::new(),
    }
}

#[test]
fn a_field_is_its_fitted_model_and_nothing_else() {
    let field = Field::fit(
        (0..8).map(|hand| observed(1_000 + hand, preflop(false), Move::Large)),
        4,
        1.0,
    )
    .expect("eight opponent-hands make a field");
    let text = field.to_text();
    assert_eq!(Field::from_text(&text), Some(field.clone()));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("svanbot field 1"));
    assert_eq!(lines.next(), Some("sides 8"));
    // Everything else is the model document, whose header is fixed, and numbers: the fit carries
    // no name, seat or hand.
    for line in lines {
        if line == "svanbot action model 1" {
            continue;
        }
        for number in line.split_whitespace() {
            number
                .parse::<f64>()
                .expect("a field line holds only action statistics");
        }
    }
    assert!(!text.contains("1000"), "a hand id crossed into the field");
}

#[test]
fn one_bots_raising_hands_and_another_bots_folding_hands_make_different_fields() {
    let raises = Field::fit(
        (0..200).map(|hand| observed(hand, preflop(false), Move::Large)),
        8,
        1.0,
    )
    .unwrap();
    let folds = Field::fit(
        (0..200).map(|hand| observed(hand, preflop(true), Move::Fold)),
        8,
        1.0,
    )
    .unwrap();
    assert_ne!(raises.model(), folds.model());

    let open = spot("AsKs", None);
    let raised = (0..64)
        .filter(|&seed| matches!(raises.decide(&open, seed), Action::RaiseTo(_)))
        .count();
    assert!(raised > 48, "the field that raised should raise: {raised}");

    let facing = spot("AsKs", Some(40));
    let raised_field_folds = (0..64)
        .filter(|&seed| raises.decide(&facing, seed) == Action::Fold)
        .count();
    let folded = (0..64)
        .filter(|&seed| folds.decide(&facing, seed) == Action::Fold)
        .count();
    assert!(
        folded > 48,
        "the field that folded should fold: {folded} of 64"
    );
    assert!(
        raised_field_folds * 2 < folded,
        "the raising field folds {raised_field_folds} to the folding field's {folded}"
    );
}

#[test]
fn a_field_row_is_named_apart_from_the_release_rows() {
    assert_eq!(row_name("SurSvan"), "field SurSvan");
    assert!(is_row(&row_name("SurSvan")));
    assert!(apart(&row_name("SurSvan")));
    assert!(apart("stablemate tight"));
    assert!(!apart("pricer"));
    assert!(!apart(svanbot_sim::v10::seat::NAME));
}
