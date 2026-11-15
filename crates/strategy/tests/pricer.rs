//! The pricer: the fallback way to decide. It prices each legal action by hero's equity against
//! the opponents' ranges (uniform, until V11-06 brings real ones) and takes the best.

use svanbot_core::card::{Card, CardSet};
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::pricer::{Budget, decide};

const BUDGET: Budget = Budget {
    samples: 3_200,
    seed: 5,
};

fn spot(hole: &str, board: &str, pot: u32, legal: Legal, opponents: usize) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal,
        opponents,
        stack: 2000,
    }
}

fn facing(call: u32, raise: Option<(u32, u32)>) -> Legal {
    Legal {
        call: Some(call),
        raise: raise.map(|(min, max)| RaiseRange { min, max }),
    }
}

#[test]
fn aces_raise_preflop() {
    let decision = decide(
        &spot("AhAs", "", 30, facing(20, Some((40, 2000))), 1),
        BUDGET,
    );
    assert!(
        matches!(decision.action, Action::RaiseTo(_)),
        "{decision:?}"
    );
}

#[test]
fn a_weak_hand_folds_to_a_big_bet() {
    // 7-2 offsuit on a board it missed, facing a pot-sized bet: it needs a third of the pot.
    let decision = decide(
        &spot("7c2d", "AhKsQh", 400, facing(200, Some((400, 1800))), 1),
        BUDGET,
    );
    assert_eq!(decision.action, Action::Fold, "{decision:?}");
}

#[test]
fn a_free_look_is_never_folded() {
    let free = Legal {
        call: None,
        raise: Some(RaiseRange { min: 20, max: 2000 }),
    };
    let decision = decide(&spot("7c2d", "AhKsQh", 40, free, 2), BUDGET);
    assert_eq!(decision.action, Action::Check, "{decision:?}");
}

#[test]
fn the_nuts_on_the_river_bet_for_value() {
    let free = Legal {
        call: None,
        raise: Some(RaiseRange { min: 20, max: 1500 }),
    };
    let decision = decide(&spot("AhKh", "QhJhTh2c3d", 300, free, 1), BUDGET);
    assert!(
        matches!(decision.action, Action::RaiseTo(_)),
        "{decision:?}"
    );
    assert!(decision.equity > 0.99);
}

#[test]
fn every_decision_is_legal_and_the_same_seed_repeats_it() {
    let mut rng = Rng::seeded(9);
    let deck: Vec<String> = "23456789TJQKA"
        .chars()
        .flat_map(|rank| "cdhs".chars().map(move |suit| format!("{rank}{suit}")))
        .collect();
    for _ in 0..200 {
        let board_len = [0, 3, 4, 5][rng.below(4) as usize];
        let mut seen = CardSet::EMPTY;
        let mut dealt: Vec<Card> = Vec::new();
        while dealt.len() < 2 + board_len {
            let card: Card = deck[rng.below(52) as usize].parse().unwrap();
            if seen.insert(card) {
                dealt.push(card);
            }
        }
        let (hole, board) = dealt.split_at(2);
        let to_call = [0, 20, 100, 600][rng.below(4) as usize];
        let raise = (rng.below(3) > 0).then(|| RaiseRange {
            min: to_call.max(20) * 2,
            max: 2000,
        });
        let legal = Legal {
            call: (to_call > 0).then_some(to_call),
            raise,
        };
        let spot = Spot {
            hole: hole.iter().copied().collect(),
            board: board.iter().copied().collect(),
            pot: 30 + to_call * 2,
            legal,
            opponents: 1 + rng.below(5) as usize,
            stack: 2000,
        };
        let budget = Budget {
            samples: 640,
            seed: rng.next_u64(),
        };
        let decision = decide(&spot, budget);
        let legal_now = match decision.action {
            Action::Fold => to_call > 0,
            Action::Check => to_call == 0,
            Action::Call => to_call > 0,
            Action::RaiseTo(to) => raise.is_some_and(|r| (r.min..=r.max).contains(&to)),
        };
        assert!(legal_now, "{decision:?} at {spot:?}");
        assert_eq!(decide(&spot, budget), decision);
    }
}
