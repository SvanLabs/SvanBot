//! The policy: a re-solve where one covers the spot, and the pricer everywhere else.

use svanbot_core::rules::{Action, Legal, RaiseRange, Street};
use svanbot_core::spot::{Act, ActKind, Spot};
use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{Branch, Personal, decide, decide_drawn, decide_read};
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::read::Class;
use svanbot_strategy::resolve::{Abstract, resolve};

#[test]
fn a_re_solved_decision_says_what_it_drew_from() {
    // The nuts facing a river bet of 200 into 200: folding is worth nothing more, and calling
    // wins the 400 in the middle whatever the villain holds.
    let river = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let decision = decide_drawn(&river, None, &Params::DEFAULT, BUDGET, 60);
    let drawn = decision.drawn.expect("a re-solve mixes");
    assert!((drawn.strategy.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    assert_eq!(drawn.strategy.len(), drawn.values.len());
    let resolved = resolve(&river, None, 0.0, 60, BUDGET.seed).unwrap();
    let at = |action| {
        resolved
            .strategy
            .iter()
            .position(|&(a, _)| a == action)
            .unwrap()
    };
    assert!(drawn.values[at(Abstract::Fold)].abs() < 1e-9);
    assert!((drawn.values[at(Abstract::Call)] - 400.0).abs() < 1e-6);
    assert_eq!(drawn.taken, resolved.taken);
    assert_eq!(decision.action, resolved.action);
}

#[test]
fn a_bare_action_is_a_decision_with_no_trace_behind_it() {
    // A caller that decides without the policy — a test's fixed action, a replay — has no trace
    // to offer. The decision says so with `None`, so the replayer is told which of the two it
    // holds rather than shown an empty trace as though a decision had no reasoning.
    let decision = svanbot_strategy::policy::Decision::from(Action::Fold);
    assert_eq!(decision.action, Action::Fold);
    assert_eq!(decision.trace, None);
    assert_eq!(decision.drawn, None);
}

#[test]
fn the_pricer_draws_nothing() {
    let preflop = spot("AhAs", "", 30, 20, 1);
    let decision = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(decision.drawn, None);
}

#[test]
fn a_decision_says_which_branch_made_it() {
    // The trace answers "why did it do that", and the first thing it must say is which of the two
    // ways decided: a re-solve of the rest of the street, or the pricer.
    let river = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let resolved = decide_drawn(&river, None, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(resolved.trace.unwrap().branch, Branch::Resolved);
    let preflop = spot("AhAs", "", 30, 20, 1);
    let priced = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(priced.trace.unwrap().branch, Branch::Priced);
}

const BUDGET: Budget = Budget {
    samples: 3_200,
    seed: 5,
};

fn spot(hole: &str, board: &str, pot: u64, call: u64, opponents: usize) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal: Legal {
            call: Some(call),
            raise: Some(RaiseRange {
                min: 2 * call,
                max: 2000,
            }),
        },
        opponents,
        stack: 2000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        lines: vec![],
        seats: vec![],
    }
}

#[test]
fn a_heads_up_spot_after_the_flop_plays_the_re_solve() {
    // A royal flush facing a pot-sized river bet: the pricer would do the same every time, the
    // re-solve mixes, and the policy plays what the re-solve drew.
    let river = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let resolved = resolve(&river, None, 0.0, 60, BUDGET.seed).unwrap();
    assert_eq!(
        decide(&river, None, &Params::DEFAULT, BUDGET, 60),
        resolved.action
    );
    assert_ne!(
        decide(&river, None, &Params::DEFAULT, BUDGET, 60),
        Action::Fold
    );
}

#[test]
fn the_pricer_decides_before_the_flop_and_in_a_pot_of_three_or_more() {
    let preflop = spot("AhAs", "", 30, 20, 1);
    let multiway = spot("7c2d", "AhKsQh", 400, 200, 2);
    for spot in [preflop, multiway] {
        assert_eq!(
            decide(&spot, None, &Params::DEFAULT, BUDGET, 60),
            pricer::decide(&spot, None, BUDGET).action
        );
    }
}

/// A model in which a river bet is called only with the strongest bucket.
fn calls_only_the_nuts() -> ActionModel {
    let facing = Situation {
        street: Street::River,
        facing: true,
    };
    let mut counts = Counts::default();
    for bucket in 0..BUCKETS {
        let mv = if bucket == BUCKETS - 1 {
            Move::Call
        } else {
            Move::Fold
        };
        counts.add(facing, bucket, mv, 50.0);
    }
    ActionModel::from_counts(&counts, 0.01)
}

#[test]
fn each_opponent_is_read_by_their_own_model_and_the_trace_names_their_class_and_trust() {
    // Two opponents called a river bet: the pricer's spot. Seat 3 has a read that calls only the
    // nuts; seat 1 has none and is read by the population's model.
    let board = "AhKs9d4c2s";
    let mut spot = spot("7c7d", board, 600, 200, 2);
    let called = Act {
        street: Street::River,
        board: board.parse().unwrap(),
        facing: true,
        kind: ActKind::Call,
    };
    spot.lines = vec![vec![called], vec![called]];
    spot.seats = vec![1, 3];
    let personal = [Personal {
        seat: 3,
        model: calls_only_the_nuts(),
        class: Class::Fixed,
        trust: Some(0.62),
    }];
    let decision = decide_read(&spot, None, &personal, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.unwrap();
    let priced = trace.priced.unwrap();
    assert_eq!(priced.exploit, Some(Params::DEFAULT.exploit));
    let ranges = priced.ranges;
    let (one, three) = (&ranges[0], &ranges[1]);
    assert_eq!((one.seat, one.class, one.trust), (Some(1), None, None));
    assert_eq!(
        (three.seat, three.class, three.trust),
        (Some(3), Some(Class::Fixed), Some(0.62))
    );
    // The read puts seat 3's range in the top bucket; the population's reads seat 1 wider.
    assert!(three.masses[BUCKETS - 1] > 0.9, "{:?}", three.masses);
    assert!(one.masses[BUCKETS - 1] < 0.5, "{:?}", one.masses);
    // With no reads, it decides as before.
    assert_eq!(
        decide_read(&spot, None, &[], &Params::DEFAULT, BUDGET, 60),
        decide_drawn(&spot, None, &Params::DEFAULT, Budget { ..BUDGET }, 60)
    );
}

#[test]
fn a_read_that_folds_to_any_bet_is_bet_into_at_a_table_of_three() {
    // Two opponents checked a flop our 7-2 missed. Read by the population's model they call
    // a bet often enough that we check; read as players who fold to any bet, we bet.
    let board = "AhKsQh";
    let mut spot = spot("7c2d", board, 400, 0, 2);
    spot.legal = Legal {
        call: None,
        raise: Some(RaiseRange { min: 20, max: 2000 }),
    };
    let checked = Act {
        street: Street::Flop,
        board: board.parse().unwrap(),
        facing: false,
        kind: ActKind::Check,
    };
    spot.lines = vec![vec![checked], vec![checked]];
    spot.seats = vec![1, 3];
    let mut counts = Counts::default();
    for situation in Situation::all() {
        for bucket in 0..BUCKETS {
            let mv = if situation.facing {
                Move::Fold
            } else {
                Move::Check
            };
            counts.add(situation, bucket, mv, 1.0);
        }
    }
    let folds = ActionModel::from_counts(&counts, 0.0);
    let personal = [1, 3].map(|seat| Personal {
        seat,
        model: folds.clone(),
        class: Class::Fixed,
        trust: Some(0.5),
    });
    let plain = decide_read(&spot, None, &[], &Params::DEFAULT, BUDGET, 60);
    let read = decide_read(&spot, None, &personal, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(plain.action, Action::Check, "{plain:?}");
    assert!(matches!(read.action, Action::RaiseTo(_)), "{read:?}");
}

#[test]
fn a_held_read_decides_exactly_as_no_read_and_the_trace_names_only_its_class() {
    // Reads that have not cleared their gate: each line is read by the population's model, so the
    // decision is the one with no reads at all, bit for bit. The trace still names each class.
    let board = "AhKs9d4c2s";
    let mut spot = spot("7c7d", board, 600, 200, 2);
    let called = Act {
        street: Street::River,
        board: board.parse().unwrap(),
        facing: true,
        kind: ActKind::Call,
    };
    spot.lines = vec![vec![called], vec![called]];
    spot.seats = vec![1, 3];
    let held = [1, 3].map(|seat| Personal {
        seat,
        model: ActionModel::baseline(),
        class: Class::Fixed,
        trust: None,
    });
    let read = decide_read(&spot, None, &held, &Params::DEFAULT, BUDGET, 60);
    let none = decide_drawn(&spot, None, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(read.action, none.action);
    let (read, none) = (
        read.trace.unwrap().priced.unwrap(),
        none.trace.unwrap().priced.unwrap(),
    );
    assert_eq!(read.equity.to_bits(), none.equity.to_bits());
    assert_eq!(read.price, none.price);
    // Nothing exploited: a held read spends none of the budget.
    assert_eq!(read.exploit, None);
    for (held, plain) in read.ranges.iter().zip(&none.ranges) {
        assert_eq!(held.masses, plain.masses);
        assert_eq!((held.class, held.trust), (Some(Class::Fixed), None));
    }
}
