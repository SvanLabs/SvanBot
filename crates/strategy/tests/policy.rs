//! The policy: a re-solve where one covers the spot, and the pricer everywhere else.

use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{Branch, decide, decide_drawn};
use svanbot_strategy::pricer::{self, Budget};
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
    let solved = decide_drawn(&river, None, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(solved.trace.unwrap().branch, Branch::Solved);
    let preflop = spot("AhAs", "", 30, 20, 1);
    let priced = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    assert_eq!(priced.trace.unwrap().branch, Branch::Priced);
}

const BUDGET: Budget = Budget {
    samples: 3_200,
    seed: 5,
};

fn spot(hole: &str, board: &str, pot: u32, call: u32, opponents: usize) -> Spot {
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
        committed: 0,
        big_blind: 20,
        in_position: false,
        lines: vec![],
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
