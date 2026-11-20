//! The re-solve: a depth-limited solve of the rest of the street, heads up after the flop.

use svanbot_core::card::CardSet;
use svanbot_core::eval::strength;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::combos;
use svanbot_strategy::resolve::{
    Abstract, Continuation, ITERATIONS, Resolved, continued_against, iterations_for, resolve,
    showdown_against,
};

const SEED: u64 = 5;

fn spot(hole: &str, board: &str, pot: u32, legal: Legal) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal,
        opponents: 1,
        stack: 2000,
        committed: 0,
        big_blind: 20,
        in_position: false,
        lines: vec![],
        seats: vec![],
    }
}

fn facing(call: u32, raise: Option<(u32, u32)>) -> Legal {
    Legal {
        call: Some(call),
        raise: raise.map(|(min, max)| RaiseRange { min, max }),
    }
}

/// How often the root strategy plays an action that `pick` accepts.
fn frequency(resolved: &Resolved, pick: impl Fn(Abstract) -> bool) -> f64 {
    resolved
        .strategy
        .iter()
        .filter(|(action, _)| pick(*action))
        .map(|(_, p)| p)
        .sum()
}

#[test]
fn only_a_heads_up_spot_after_the_flop_is_re_solved() {
    let preflop = spot("AhAs", "", 30, facing(20, Some((40, 2000))));
    assert_eq!(resolve(&preflop, None, 0.0, 50, SEED), None);
    let mut multiway = spot("AhAs", "Ks9d4c", 60, facing(20, Some((40, 2000))));
    multiway.opponents = 2;
    assert_eq!(resolve(&multiway, None, 0.0, 50, SEED), None);
}

#[test]
fn the_nuts_never_fold_on_the_river() {
    // A royal flush, facing a pot-sized bet.
    let river = spot("AhKh", "QhJhTh2c3d", 400, facing(200, Some((400, 2000))));
    let resolved = resolve(&river, None, 0.0, 200, SEED).unwrap();
    assert!(
        frequency(&resolved, |a| a == Abstract::Fold) < 0.01,
        "{resolved:?}"
    );
    assert_ne!(resolved.action, Action::Fold);
}

#[test]
fn seven_high_does_not_call_a_pot_sized_river_bet() {
    let river = spot("7c2d", "AhKsQh9s4d", 400, facing(200, Some((400, 2000))));
    let resolved = resolve(&river, None, 0.0, 200, SEED).unwrap();
    assert!(
        frequency(&resolved, |a| a == Abstract::Call) < 0.02,
        "{resolved:?}"
    );
}

#[test]
fn every_re_solved_action_is_legal_and_the_same_seed_repeats_it() {
    let boards = ["Ks9d4c", "Ks9d4c7h", "Ks9d4c7h2s"];
    let mut rng = Rng::seeded(3);
    for round in 0..6 {
        let pot = 40 + rng.below(600) as u32;
        let stack = 100 + rng.below(3000) as u32;
        let legal = if round % 2 == 0 {
            Legal {
                call: None,
                raise: Some(RaiseRange {
                    min: 20.min(stack),
                    max: stack,
                }),
            }
        } else {
            let call = (1 + rng.below(u64::from(pot) / 2) as u32).min(stack);
            let raise = (stack > 2 * call).then_some(RaiseRange {
                min: 2 * call,
                max: stack,
            });
            Legal {
                call: Some(call),
                raise,
            }
        };
        let mut spot = spot("AcJd", boards[round % 3], pot, legal);
        spot.stack = stack;
        let seed = rng.below(1 << 20);
        let resolved = resolve(&spot, None, 0.0, 10, seed).unwrap();
        match resolved.action {
            Action::Fold | Action::Call => assert!(legal.call.is_some(), "{spot:?}"),
            Action::Check => assert!(legal.call.is_none(), "{spot:?}"),
            Action::RaiseTo(to) => {
                let range = legal.raise.expect("a raise was offered");
                assert!((range.min..=range.max).contains(&to), "{to} at {spot:?}");
            }
        }
        let total: f64 = resolved.strategy.iter().map(|(_, p)| p).sum();
        assert!((total - 1.0).abs() < 1e-9, "{resolved:?}");
        assert_eq!(resolve(&spot, None, 0.0, 10, seed), Some(resolved));
    }
}

#[test]
fn a_bigger_pot_buys_more_iterations_between_the_floor_and_the_cap() {
    assert_eq!(iterations_for(0, 20, 5), ITERATIONS.0);
    assert!(iterations_for(400, 20, 5) > iterations_for(100, 20, 5));
    assert_eq!(iterations_for(1_000_000, 20, 5), ITERATIONS.1);
    assert_eq!(iterations_for(400, 0, 5), iterations_for(400, 1, 5));
    // Before the river an iteration costs four of the river's, so there are a quarter as many.
    assert_eq!(iterations_for(400, 20, 3), iterations_for(400, 20, 5) / 4);
    assert_eq!(iterations_for(400, 20, 4), iterations_for(400, 20, 3));
}

#[test]
fn the_showdown_sweep_matches_a_count_pair_by_pair() {
    let board: CardSet = "Ks9d4c7h2s".parse().unwrap();
    let pairs = combos(board);
    let mut rng = Rng::seeded(11);
    let reach: Vec<f64> = pairs
        .iter()
        .map(|_| f64::from(rng.below(5) as u32))
        .collect();
    let (share, compatible) = showdown_against(&pairs, board, &reach);
    for (at, &mine) in pairs.iter().enumerate() {
        let (mut won, mut live) = (0.0, 0.0);
        for (other, &theirs) in pairs.iter().enumerate() {
            if !mine.is_disjoint(theirs) {
                continue;
            }
            live += reach[other];
            won += match strength(mine | board).cmp(&strength(theirs | board)) {
                std::cmp::Ordering::Greater => reach[other],
                std::cmp::Ordering::Equal => 0.5 * reach[other],
                std::cmp::Ordering::Less => 0.0,
            };
        }
        assert!(
            (share[at] - won).abs() < 1e-9,
            "pair {at}: {} vs {won}",
            share[at]
        );
        assert!((compatible[at] - live).abs() < 1e-9, "pair {at}");
    }
}

#[test]
fn top_pair_folds_more_to_a_player_who_bets_big_only_when_strong() {
    use svanbot_core::rules::Street;
    use svanbot_core::spot::{Act, ActKind};
    use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
    // The bettor bets the pot on the river only with the top eighth of their range.
    let mut counts = Counts::default();
    let open = Situation {
        street: Street::River,
        facing: false,
    };
    for bucket in 0..BUCKETS {
        let mv = if bucket == BUCKETS - 1 {
            Move::Large
        } else {
            Move::Check
        };
        counts.add(open, bucket, mv, 200.0);
    }
    let polar = ActionModel::from_counts(&counts, 1.0);
    let board = "Ks9d4c7h2s";
    let mut river = spot("KcJd", board, 200, facing(100, None));
    river.lines = vec![vec![Act {
        street: Street::River,
        board: board.parse().unwrap(),
        facing: false,
        kind: ActKind::Bet {
            added: 100,
            pot: 100,
        },
    }]];
    let fold = |model| {
        let resolved = resolve(&river, model, 0.0, 150, SEED).unwrap();
        frequency(&resolved, |a| a == Abstract::Fold)
    };
    let (any_two, strong) = (fold(None), fold(Some(&polar)));
    assert!(any_two < 0.2, "against any two cards: {any_two}");
    assert!(strong > 0.8, "against the strong range: {strong}");
}

#[test]
fn a_flush_draw_on_the_turn_is_worth_the_cards_to_come() {
    // Three high facing a fifth of the pot: the call needs one win in seven. With two hearts on
    // a board with two, nine of the 46 rivers make a flush. Without the draw there is no hand.
    let non_fold = |hole| {
        let turn = spot(hole, "Kh9h4c7s", 240, facing(40, Some((80, 2000))));
        let resolved = resolve(&turn, None, 0.0, 60, SEED).unwrap();
        1.0 - frequency(&resolved, |a| a == Abstract::Fold)
    };
    let (draw, nothing) = (non_fold("3h2h"), non_fold("3c2d"));
    assert!(draw > 0.7, "the draw continues {draw}");
    assert!(nothing < draw - 0.3, "no draw continues {nothing}");
}

#[test]
fn with_no_fitted_model_a_raise_still_narrows_the_villain_to_strong_hands() {
    use svanbot_core::rules::Street;
    use svanbot_core::spot::{Act, ActKind};
    // Second pair on an ace-high flop. The villain bet, we raised, and they shoved over it.
    let board = "2c6cAs";
    let mut flop = spot("6dKd", board, 770, facing(350, None));
    flop.committed = 185;
    flop.stack = 350;
    let act = |facing, added, pot| Act {
        street: Street::Flop,
        board: board.parse().unwrap(),
        facing,
        kind: ActKind::Bet { added, pot },
    };
    let fold = |spot: &Spot| {
        let resolved = resolve(spot, None, 0.0, 100, SEED).unwrap();
        frequency(&resolved, |a| a == Abstract::Fold)
    };
    // With the hand not followed there is no line, and the villain could hold anything.
    let unseen = fold(&flop);
    flop.lines = vec![vec![act(false, 50, 40), act(true, 485, 285)]];
    let raised = fold(&flop);
    assert!(raised > 0.7, "facing the shove it folds {raised}");
    assert!(
        unseen < raised - 0.3,
        "against any two cards it folds {unseen}"
    );
}

#[test]
fn the_continuation_round_matches_a_count_pair_by_pair() {
    use svanbot_strategy::action::{BUCKETS, buckets};
    let board: CardSet = "Ks9d4c7h2s".parse().unwrap();
    let pairs = combos(board);
    let bucket: Vec<usize> = buckets(board, CardSet::EMPTY)
        .into_iter()
        .map(|(_, bucket)| bucket)
        .collect();
    let mut rng = Rng::seeded(17);
    let mut chance = || rng.below(1000) as f64 / 1000.0;
    let reach: Vec<f64> = pairs.iter().map(|_| chance()).collect();
    let mut continuation = || Continuation {
        bet: std::array::from_fn::<_, BUCKETS, _>(|_| chance()),
        call: std::array::from_fn::<_, BUCKETS, _>(|_| chance()),
    };
    let (mine, theirs) = (continuation(), continuation());
    let (pot, paid, bet) = (300.0, 100.0, 225.0);
    for first in [true, false] {
        let got = continued_against(&pairs, board, &reach, mine, theirs, first, (pot, paid, bet));
        for (at, &hole) in pairs.iter().enumerate().step_by(7) {
            let (b, mut want) = (bucket[at], 0.0);
            for (other, &held) in pairs.iter().enumerate() {
                if !hole.is_disjoint(held) {
                    continue;
                }
                let o = bucket[other];
                let win = match strength(hole | board).cmp(&strength(held | board)) {
                    std::cmp::Ordering::Greater => 1.0,
                    std::cmp::Ordering::Equal => 0.5,
                    std::cmp::Ordering::Less => 0.0,
                };
                let small = pot * win - paid;
                let big = (pot + 2.0 * bet) * win - paid - bet;
                let value = if first {
                    (1.0 - mine.bet[b])
                        * ((1.0 - theirs.bet[o]) * small
                            + theirs.bet[o] * ((1.0 - mine.call[b]) * -paid + mine.call[b] * big))
                        + mine.bet[b]
                            * ((1.0 - theirs.call[o]) * (pot - paid) + theirs.call[o] * big)
                } else {
                    (1.0 - theirs.bet[o])
                        * ((1.0 - mine.bet[b]) * small
                            + mine.bet[b]
                                * ((1.0 - theirs.call[o]) * (pot - paid) + theirs.call[o] * big))
                        + theirs.bet[o] * ((1.0 - mine.call[b]) * -paid + mine.call[b] * big)
                };
                want += reach[other] * value;
            }
            assert!(
                (got[at] - want).abs() < 1e-6,
                "pair {at}, first {first}: {} vs {want}",
                got[at]
            );
        }
    }
}

#[test]
fn nothing_floats_a_villain_held_to_folding_the_river_and_not_one_free_to_answer() {
    use svanbot_core::rules::Street;
    use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
    // On the river this villain never bets and folds to every bet.
    let mut counts = Counts::default();
    for bucket in 0..BUCKETS {
        let river = |facing| Situation {
            street: Street::River,
            facing,
        };
        counts.add(river(false), bucket, Move::Check, 500.0);
        counts.add(river(true), bucket, Move::Fold, 500.0);
    }
    let folder = ActionModel::from_counts(&counts, 1.0);
    // Three high with no draw, facing half the pot on the turn. It cannot win a showdown; only
    // a river bet that takes the pot makes a call now worth it.
    let turn = spot("3c2d", "Kh9h4c7s", 300, facing(100, Some((200, 2000))));
    let continues = |exploit| {
        let resolved = resolve(&turn, Some(&folder), exploit, 60, SEED).unwrap();
        1.0 - frequency(&resolved, |a| a == Abstract::Fold)
    };
    let (held, free) = (continues(1.0), continues(0.0));
    assert!(
        held > 0.7,
        "against a villain held to folding it continues {held}"
    );
    assert!(free < held - 0.2, "against one free to answer, {free}");
}
