//! The action model: a probability for every move at every situation and strength bucket, and the
//! range that follows from it by Bayes' rule.

use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_strategy::action::{
    ActionModel, BUCKETS, Counts, Move, Seen, Situation, buckets, range,
};

fn cards(text: &str) -> CardSet {
    text.parse().unwrap()
}

fn total(range: &[(CardSet, f64)]) -> f64 {
    range.iter().map(|(_, weight)| weight).sum()
}

fn weight_of(range: &[(CardSet, f64)], combo: &str) -> f64 {
    let combo = cards(combo);
    range
        .iter()
        .find(|(held, _)| *held == combo)
        .map_or(0.0, |(_, weight)| *weight)
}

fn river(facing: bool) -> Situation {
    Situation {
        street: Street::River,
        facing,
    }
}

/// A model where the top bucket bets big on the river and the bottom bucket checks.
fn polar() -> ActionModel {
    let mut counts = Counts::default();
    let open = river(false);
    for bucket in 0..BUCKETS {
        let strong = bucket == BUCKETS - 1;
        counts.add(
            open,
            bucket,
            if strong { Move::Large } else { Move::Check },
            100.0,
        );
    }
    ActionModel::from_counts(&counts, 1.0)
}

#[test]
fn every_situation_and_bucket_gives_a_distribution_over_its_own_moves() {
    for model in [ActionModel::uniform(), polar()] {
        for situation in Situation::all() {
            for bucket in 0..BUCKETS {
                let sum: f64 = situation
                    .moves()
                    .iter()
                    .map(|&mv| model.prob(situation, bucket, mv))
                    .sum();
                assert!((sum - 1.0).abs() < 1e-12, "{situation:?} {bucket}: {sum}");
            }
        }
    }
    // A fold is no move when the check is free, and a check none when facing a bet.
    assert!(!river(false).moves().contains(&Move::Fold));
    assert!(!river(true).moves().contains(&Move::Check));
    assert_eq!(river(false).counted(Move::Fold), Move::Check);
    assert_eq!(river(true).counted(Move::Check), Move::Call);
}

#[test]
fn a_bet_is_sized_by_the_chips_it_adds_to_the_pot() {
    assert_eq!(Move::bet(20, 100), Move::Small);
    assert_eq!(Move::bet(60, 100), Move::Medium);
    assert_eq!(Move::bet(100, 100), Move::Large);
    assert_eq!(Move::bet(5000, 100), Move::Large);
}

#[test]
fn buckets_order_the_live_combos_by_strength_and_skip_the_dead_cards() {
    let board = cards("AhKhQh7c2d");
    let dead = cards("3s3d");
    let ranked = buckets(board, dead);
    assert_eq!(ranked.len(), 45 * 44 / 2);
    assert!(
        ranked
            .iter()
            .all(|(combo, _)| combo.is_disjoint(board | dead))
    );
    let bucket = |combo| ranked.iter().find(|(c, _)| *c == cards(combo)).unwrap().1;
    assert_eq!(bucket("JhTh"), BUCKETS - 1, "the royal flush");
    assert_eq!(bucket("5c4s"), 0);
    // A set of aces outranks ace-king-queen-nine-eight high. (Two pair is in the top eighth too.)
    assert!(bucket("AsAd") > bucket("9c8c"));
    // Preflop, by the cards alone.
    let preflop = buckets(CardSet::EMPTY, CardSet::EMPTY);
    assert_eq!(preflop.len(), 1326);
    let before = |combo| preflop.iter().find(|(c, _)| *c == cards(combo)).unwrap().1;
    assert_eq!(before("AsAc"), BUCKETS - 1);
    assert_eq!(before("7c2d"), 0);
    assert!(before("AsKs") > before("9c4d"));
}

#[test]
fn the_uniform_model_learns_nothing_from_an_action() {
    let seen = [Seen {
        board: cards("AhKhQh7c2d"),
        situation: river(false),
        mv: Move::Large,
    }];
    let range = range(&ActionModel::uniform(), &seen, cards("3s3d"));
    assert_eq!(range.len(), 45 * 44 / 2);
    assert!((total(&range) - 1.0).abs() < 1e-9);
    assert!((weight_of(&range, "JhTh") - weight_of(&range, "5c4s")).abs() < 1e-15);
}

#[test]
fn a_big_river_bet_moves_the_range_to_the_strong_combos_and_a_check_away_from_them() {
    let board = cards("AhKhQh7c2d");
    let saw = |mv| {
        [Seen {
            board,
            situation: river(false),
            mv,
        }]
    };
    let model = polar();
    let bet = range(&model, &saw(Move::Large), cards("3s3d"));
    let check = range(&model, &saw(Move::Check), cards("3s3d"));
    assert!((total(&bet) - 1.0).abs() < 1e-9 && (total(&check) - 1.0).abs() < 1e-9);
    assert!(weight_of(&bet, "JhTh") > 50.0 * weight_of(&bet, "5c4s"));
    assert!(weight_of(&check, "JhTh") < weight_of(&check, "5c4s") / 50.0);
    // A dead card is in no combo of the range.
    assert_eq!(weight_of(&bet, "3s4s"), 0.0);
}

#[test]
fn the_baseline_model_bets_and_continues_more_the_stronger_the_cards() {
    use svanbot_strategy::action::{ActionModel, BUCKETS, Move, Situation};
    let model = ActionModel::baseline();
    for situation in Situation::all() {
        let aggression = |bucket| {
            [Move::Small, Move::Medium, Move::Large]
                .map(|mv| model.prob(situation, bucket, mv))
                .iter()
                .sum::<f64>()
        };
        // The weakest bucket bluffs; from the second up, aggression only grows.
        for bucket in 2..BUCKETS {
            assert!(
                aggression(bucket) >= aggression(bucket - 1),
                "{situation:?}"
            );
        }
        assert!(
            aggression(BUCKETS - 1) > 3.0 * aggression(1),
            "{situation:?}"
        );
        assert!(aggression(0) > aggression(1), "{situation:?}");
        if situation.facing {
            for bucket in 1..BUCKETS {
                let fold = |bucket| model.prob(situation, bucket, Move::Fold);
                assert!(fold(bucket) <= fold(bucket - 1), "{situation:?}");
            }
        }
        for bucket in 0..BUCKETS {
            let total: f64 = situation
                .moves()
                .iter()
                .map(|&mv| model.prob(situation, bucket, mv))
                .sum();
            assert!((total - 1.0).abs() < 1e-9);
        }
    }
}
