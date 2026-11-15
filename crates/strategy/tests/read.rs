//! Reads: the population model tilted toward one opponent's own frequencies, as far as the
//! evidence and the exploit budget allow.

use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Seen, Situation};
use svanbot_strategy::read::{Class, Read, Trust, personal};

fn river_facing() -> Situation {
    Situation {
        street: Street::River,
        facing: true,
    }
}

/// A population that folds the weak half to a river bet and calls the strong half.
fn population() -> ActionModel {
    let mut counts = Counts::default();
    for bucket in 0..BUCKETS {
        let mv = if bucket < BUCKETS / 2 {
            Move::Fold
        } else {
            Move::Call
        };
        counts.add(river_facing(), bucket, mv, 50.0);
    }
    ActionModel::from_counts(&counts, 1.0)
}

/// A read of `folds` folds in `n` river bets faced.
fn folder(folds: u32, n: u32) -> Read {
    let mut read = Read::default();
    for i in 0..n {
        read.observe(&Seen {
            board: CardSet::EMPTY,
            situation: river_facing(),
            mv: if i < folds { Move::Fold } else { Move::Call },
        });
    }
    read
}

fn fold_at(model: &ActionModel, bucket: usize) -> f64 {
    model.prob(river_facing(), bucket, Move::Fold)
}

#[test]
fn no_evidence_or_no_budget_leaves_the_population_model() {
    let population = population();
    assert_eq!(
        personal(&population, &Read::default(), Trust::DEFAULT),
        population
    );
    let none = Trust {
        budget: 0.0,
        ..Trust::DEFAULT
    };
    assert_eq!(personal(&population, &folder(200, 200), none), population);
}

#[test]
fn a_player_who_always_folds_folds_more_in_every_bucket_and_rows_stay_distributions() {
    let population = population();
    let read = personal(&population, &folder(200, 200), Trust::DEFAULT);
    for bucket in 0..BUCKETS {
        assert!(
            fold_at(&read, bucket) > fold_at(&population, bucket),
            "bucket {bucket}"
        );
        let sum: f64 = river_facing()
            .moves()
            .iter()
            .map(|&mv| read.prob(river_facing(), bucket, mv))
            .sum();
        assert!((sum - 1.0).abs() < 1e-12);
    }
}

#[test]
fn trust_grows_with_the_evidence_and_shrinks_with_the_budget() {
    let population = population();
    let shift = |read: &Read, budget: f64| {
        let trust = Trust {
            budget,
            ..Trust::DEFAULT
        };
        fold_at(&personal(&population, read, trust), BUCKETS - 1)
            - fold_at(&population, BUCKETS - 1)
    };
    let (few, many) = (folder(5, 5), folder(500, 500));
    assert!(shift(&few, 1.0) < shift(&many, 1.0));
    let damped = shift(&many, Class::Learning.budget());
    assert!(damped > 0.0 && damped < shift(&many, Class::Fixed.budget()));
}
