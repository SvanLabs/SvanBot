//! The policy: a re-solve where one covers the spot, and the pricer everywhere else.

use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::policy::decide;
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::resolve::resolve;

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
        lines: vec![],
    }
}

#[test]
fn a_heads_up_spot_after_the_flop_plays_the_re_solve() {
    // A royal flush facing a pot-sized river bet: the pricer would do the same every time, the
    // re-solve mixes, and the policy plays what the re-solve drew.
    let river = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let resolved = resolve(&river, None, 60, BUDGET.seed).unwrap();
    assert_eq!(decide(&river, None, BUDGET, 60), resolved.action);
    assert_ne!(decide(&river, None, BUDGET, 60), Action::Fold);
}

#[test]
fn the_pricer_decides_before_the_flop_and_in_a_pot_of_three_or_more() {
    let preflop = spot("AhAs", "", 30, 20, 1);
    let multiway = spot("7c2d", "AhKsQh", 400, 200, 2);
    for spot in [preflop, multiway] {
        assert_eq!(
            decide(&spot, None, BUDGET, 60),
            pricer::decide(&spot, None, BUDGET).action
        );
    }
}
