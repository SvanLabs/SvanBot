//! The three offline continuation pin treatments, using the same model and seed.

use svanbot_core::rules::{Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{ContinuationPin, Personal, Policy};
use svanbot_strategy::pricer::Budget;
use svanbot_strategy::read::Class;
use svanbot_strategy::resolve::resolve;

#[test]
fn a_continuation_pin_experiment_keeps_the_range_model_fixed() {
    let mut turn = spot("AhKh", "QhJh2c3d", 400, 80, 1);
    turn.seats = vec![1];
    turn.lines = vec![vec![]];
    let params = Params { exploit: 0.8 };
    let action_model = ActionModel::baseline();
    let policy = Policy::reference(params);
    for (pin, class, trust, expected) in [
        (ContinuationPin::Fixed, Class::Fixed, Some(0.5), 0.8),
        (ContinuationPin::Zero, Class::Fixed, Some(0.5), 0.0),
        (ContinuationPin::EvidenceCap, Class::Fixed, Some(0.5), 0.4),
        (ContinuationPin::EvidenceCap, Class::Fixed, None, 0.0),
        (
            ContinuationPin::EvidenceCap,
            Class::Switching,
            Some(0.5),
            0.0,
        ),
        (
            ContinuationPin::EvidenceCap,
            Class::Learning,
            Some(0.5),
            0.0,
        ),
    ] {
        let personal = Personal {
            seat: 1,
            model: action_model.clone(),
            class,
            trust,
        };
        let played =
            policy
                .with_continuation_pin(pin)
                .decide(&turn, None, &[personal], BUDGET, 24, &[]);
        let direct = resolve(&turn, Some(&action_model), expected, 24, BUDGET.seed).unwrap();
        assert_eq!(played.action, direct.action);
        assert_eq!(played.drawn.unwrap().values, direct.values, "{pin:?}");
    }
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
        players_behind: None,
        lines: vec![],
        seats: vec![],
    }
}
