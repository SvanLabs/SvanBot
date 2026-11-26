//! Fitting the action model on every hand by EM, with the hole cards as the hidden variable, and
//! the held-out gate a fit must pass. The hands are simulated from a known policy, so the fit can
//! be checked against the truth.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::eval::strength;
use svanbot_core::rng::Rng;
use svanbot_core::rules::Street;
use svanbot_learn::fit::{MARGIN, Observed, Outcome, fit, gate, log_likelihood, prepare};
use svanbot_strategy::action::{
    ActionModel, BUCKETS, Counts, Move, Seen, Situation, buckets, combos,
};

const STREETS: [Street; 4] = [Street::Preflop, Street::Flop, Street::Turn, Street::River];

/// The policy the hands are played by: strong hands bet big, weak ones check or fold, and the
/// middle calls.
fn truth() -> ActionModel {
    let mut counts = Counts::default();
    for situation in Situation::all() {
        for bucket in 0..BUCKETS {
            let strong = bucket >= BUCKETS - 2;
            let weak = bucket < 3;
            let (main, other) = match (situation.facing, strong, weak) {
                (_, true, _) => (Move::Large, Move::Medium),
                (true, _, true) => (Move::Fold, Move::Call),
                (true, _, _) => (Move::Call, Move::Small),
                (false, _, true) => (Move::Check, Move::Small),
                (false, _, _) => (Move::Check, Move::Medium),
            };
            counts.add(situation, bucket, main, 80.0);
            counts.add(situation, bucket, other, 20.0);
        }
    }
    ActionModel::from_counts(&counts, 1.0)
}

fn deck() -> Vec<Card> {
    Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .collect()
}

fn draw(model: &ActionModel, situation: Situation, bucket: usize, rng: &mut Rng) -> Move {
    let roll = rng.below(1 << 30) as f64 / f64::from(1u32 << 30);
    let mut sum = 0.0;
    for &mv in situation.moves() {
        sum += model.prob(situation, bucket, mv);
        if roll < sum {
            return mv;
        }
    }
    *situation.moves().last().unwrap()
}

/// One opponent's hand against us: their actions street by street, by `model`, then a showdown
/// they win and show, or lose and muck.
fn play(model: &ActionModel, hand: u64, rng: &mut Rng) -> Observed {
    let mut cards = deck();
    for i in 0..9 {
        let j = i + rng.below((cards.len() - i) as u64) as usize;
        cards.swap(i, j);
    }
    let theirs: CardSet = cards[0..2].iter().copied().collect();
    let ours: CardSet = cards[2..4].iter().copied().collect();
    let full: Vec<Card> = cards[4..9].to_vec();
    let mut seen = Vec::new();
    let mut board = CardSet::EMPTY;
    for (street, dealt) in STREETS.into_iter().zip([0, 3, 4, 5]) {
        board = full[..dealt].iter().copied().collect();
        let situation = Situation {
            street,
            facing: rng.below(2) == 0,
        };
        let ranked = buckets(board, ours);
        let bucket = ranked.iter().find(|(pair, _)| *pair == theirs).unwrap().1;
        let mv = draw(model, situation, bucket, rng);
        seen.push(Seen {
            board,
            situation,
            mv,
        });
        if mv == Move::Fold {
            return Observed {
                hand,
                seen,
                dead: ours | board,
                outcome: Outcome::Unseen,
            };
        }
    }
    let (them, us) = (strength(theirs | board), strength(ours | board));
    let outcome = if them >= us {
        Outcome::Shown(theirs)
    } else {
        Outcome::Mucked { board, winner: us }
    };
    Observed {
        hand,
        seen,
        dead: ours | board,
        outcome,
    }
}

fn hands(count: u64, seed: u64) -> Vec<Observed> {
    let model = truth();
    let mut rng = Rng::seeded(seed);
    (0..count)
        .map(|hand| play(&model, hand, &mut rng))
        .collect()
}

#[test]
fn each_round_of_em_raises_the_likelihood_and_the_fit_finds_the_policy() {
    let train: Vec<_> = hands(800, 1).iter().map(prepare).collect();
    let fitted = fit(&train, 12, 1.0);
    assert!(
        fitted
            .trace
            .windows(2)
            .all(|pair| pair[1] >= pair[0] - 1e-9),
        "{:?}",
        fitted.trace
    );
    // Every hand acts preflop, so its buckets are evenly filled; later streets thin out as the
    // weak hands fold, and their cells stay near the prior.
    let preflop_open = Situation {
        street: Street::Preflop,
        facing: false,
    };
    let truth = truth();
    for bucket in [BUCKETS - 1, 0] {
        for mv in [Move::Large, Move::Check] {
            let (got, want) = (
                fitted.model.prob(preflop_open, bucket, mv),
                truth.prob(preflop_open, bucket, mv),
            );
            assert!(
                (got - want).abs() < 0.2,
                "bucket {bucket} {mv:?}: {got} vs {want}"
            );
        }
    }
}

#[test]
fn a_fit_beats_the_uniform_model_on_held_out_hands_and_the_uniform_model_does_not() {
    let all = hands(1200, 2);
    let (train, held_out) = all.split_at(900);
    let train: Vec<_> = train.iter().map(prepare).collect();
    let held_out: Vec<_> = held_out.iter().map(prepare).collect();
    let fitted = fit(&train, 10, 1.0).model;
    let verdict = gate(&fitted, &ActionModel::uniform(), &held_out, MARGIN);
    assert!(verdict.passed, "{verdict:?}");
    assert!(verdict.lower > MARGIN && verdict.gain > verdict.lower);
    assert_eq!(verdict.hands, 300);
    let tie = gate(
        &ActionModel::uniform(),
        &ActionModel::uniform(),
        &held_out,
        MARGIN,
    );
    assert!(!tie.passed && tie.gain.abs() < 1e-12, "{tie:?}");
}

#[test]
fn the_fit_gate_stays_finite_when_the_baseline_expects_no_folds_from_aces() {
    let held_out: Vec<_> = (0..2)
        .map(|hand| {
            prepare(&Observed {
                hand,
                seen: vec![Seen {
                    board: CardSet::EMPTY,
                    situation: Situation {
                        street: Street::Preflop,
                        facing: true,
                    },
                    mv: Move::Fold,
                }],
                dead: CardSet::EMPTY,
                outcome: Outcome::Shown("AsAd".parse().unwrap()),
            })
        })
        .collect();
    let verdict = gate(
        &ActionModel::uniform(),
        &ActionModel::baseline(),
        &held_out,
        MARGIN,
    );
    assert!(
        verdict.gain.is_finite() && verdict.lower.is_finite(),
        "{verdict:?}"
    );
    assert!(verdict.passed);
}

#[test]
fn a_mucked_hand_could_only_be_a_pair_that_lost() {
    let board: CardSet = "AhKhQh7c2d".parse().unwrap();
    let ours: CardSet = "3s3d".parse().unwrap();
    let winner = strength("AsKs".parse::<CardSet>().unwrap() | board);
    let mucked = Observed {
        hand: 0,
        seen: vec![],
        dead: ours | board,
        outcome: Outcome::Mucked { board, winner },
    };
    let losers = combos(ours | board)
        .into_iter()
        .filter(|&pair| strength(pair | board) < winner)
        .count();
    assert_eq!(prepare(&mucked).candidates(), losers);
    assert!(losers > 0 && losers < 990);
    // Its likelihood under the uniform model is the share of pairs that lose.
    let ll = log_likelihood(&ActionModel::uniform(), &prepare(&mucked));
    assert!((ll - (losers as f64 / 990.0).ln()).abs() < 1e-9);
}
