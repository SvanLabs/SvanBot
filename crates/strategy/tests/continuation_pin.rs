//! The three offline continuation pin treatments, using the same model and seed.

use svanbot_core::rules::{Legal, RaiseRange, Street};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::{ActionModel, Move, Seen, Situation};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{ContinuationPin, Personal, Policy};
use svanbot_strategy::pricer::Budget;
use svanbot_strategy::read::{Class, Read};
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
            read: None,
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

#[test]
fn an_evidence_cap_uses_the_next_streets_own_evidence_on_the_flop_and_turn() {
    let action_model = ActionModel::baseline();
    let params = Params { exploit: 0.8 };
    let policy = Policy::reference(params).with_continuation_pin(ContinuationPin::EvidenceCap);
    for (board, next) in [("QhJh2c", Street::Turn), ("QhJh2c3d", Street::River)] {
        let mut at = spot("AhKh", board, 400, 80, 1);
        at.seats = vec![1];
        at.lines = vec![vec![]];
        // Only the facing answer is supported: the cap spends nothing.
        let mut facing_only = Read::default();
        for _ in 0..30 {
            facing_only.observe(&seen(next, true, Move::Call));
        }
        let played = policy.decide(
            &at,
            None,
            &[personal(&action_model, &facing_only)],
            BUDGET,
            24,
            &[],
        );
        let direct = resolve(&at, Some(&action_model), 0.0, 24, BUDGET.seed).unwrap();
        assert_eq!(played.action, direct.action);
        assert_eq!(played.drawn.unwrap().values, direct.values, "{board}");

        // Both answers are supported: the cap is the lower of their earned shares.
        let mut both = facing_only.clone();
        for _ in 0..30 {
            both.observe(&seen(next, false, Move::Check));
        }
        let played = policy.decide(
            &at,
            None,
            &[personal(&action_model, &both)],
            BUDGET,
            24,
            &[],
        );
        let direct = resolve(&at, Some(&action_model), 0.4, 24, BUDGET.seed).unwrap();
        assert_eq!(played.action, direct.action);
        assert_eq!(played.drawn.unwrap().values, direct.values, "{board}");
    }
}

#[test]
fn a_river_decision_ignores_the_continuation_pin() {
    let mut river = spot("AhKh", "QhJh2c3d4s", 400, 80, 1);
    river.seats = vec![1];
    river.lines = vec![vec![]];
    let action_model = ActionModel::baseline();
    let policy = Policy::reference(Params { exploit: 0.8 });
    let direct = resolve(&river, Some(&action_model), 0.8, 24, BUDGET.seed).unwrap();
    for pin in [
        ContinuationPin::Fixed,
        ContinuationPin::Zero,
        ContinuationPin::EvidenceCap,
    ] {
        let played = policy
            .with_continuation_pin(pin)
            .decide(&river, None, &[], BUDGET, 24, &[]);
        assert_eq!(played.action, direct.action, "{pin:?}");
        assert_eq!(played.drawn.unwrap().values, direct.values, "{pin:?}");
    }
}

fn personal(model: &ActionModel, read: &Read) -> Personal {
    Personal {
        seat: 1,
        model: model.clone(),
        class: Class::Fixed,
        trust: Some(0.5),
        read: Some(read.clone()),
    }
}

fn seen(street: Street, facing: bool, mv: Move) -> Seen {
    Seen {
        board: match street {
            Street::Turn => "QhJh2c".parse().unwrap(),
            _ => "QhJh2c3d".parse().unwrap(),
        },
        situation: Situation { street, facing },
        mv,
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
