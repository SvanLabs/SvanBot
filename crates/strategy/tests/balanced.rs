//! The `Balanced` kind: the pricer draws among near-best actions, the opening boundary is a
//! chance rather than a cut, reads are held, no profile applies, and the exploit budget is capped.

use svanbot_core::rules::{Action, Legal, RaiseRange, Street};
use svanbot_core::spot::{Act, ActKind, Spot};
use svanbot_strategy::action::{ActionModel, BUCKETS, Counts, Move, Situation};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{Personal, Policy};
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::read::Class;
use svanbot_strategy::style;

const BUDGET: Budget = Budget {
    samples: 3_200,
    seed: 5,
};

fn params(spread: f64) -> Params {
    Params {
        exploit: 0.25,
        spread,
    }
}

fn spot(hole: &str, board: &str, pot: u64, call: u64, opponents: usize) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal: Legal {
            call: Some(call),
            raise: Some(RaiseRange {
                min: 2 * call,
                max: 2_000,
            }),
        },
        opponents,
        stack: 2_000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        players_behind: None,
        lines: vec![],
        seats: vec![],
    }
}

#[test]
fn the_balanced_kind_and_the_anchor_style_are_named_and_selectable() {
    assert_eq!(PolicyKind::named("balanced"), Some(PolicyKind::Balanced));
    assert_eq!(
        style::named("anchor").map(|style| style.name),
        Some("anchor")
    );
    assert_eq!(
        Policy::new(PolicyKind::Balanced, &style::ANCHOR, Params::DEFAULT).name(),
        "balanced/anchor"
    );
    assert!(style::ANCHOR.profile.is_empty());
}

#[test]
fn anchor_seeds_its_mixing_spread_and_caps_the_exploit_budget() {
    let seeded = style::ANCHOR.seed(Params::DEFAULT);
    assert_eq!(seeded.exploit, 0.25);
    assert_eq!(seeded.spread, 0.1);
    // A slot may keep a learned spread inside the style's bounds.
    let learned = style::ANCHOR.seed(Params {
        exploit: 0.9,
        spread: 0.4,
    });
    assert_eq!(learned.exploit, 0.25);
    assert_eq!(learned.spread, 0.4);
    assert!(style::ANCHOR.allows("spread", 0.5));
    assert!(!style::ANCHOR.allows("spread", 0.6));
    assert!(!style::ANCHOR.allows("exploit", 0.3));
}

#[test]
fn the_exploit_cap_is_real_even_when_the_parameter_set_is_not_seeded() {
    let balanced = Policy::new(
        PolicyKind::Balanced,
        &style::ANCHOR,
        Params {
            exploit: 1.0,
            spread: 0.1,
        },
    );
    assert_eq!(balanced.exploit(), 0.25);
    // The capped number is the one the priced trace names, even when a model is in play.
    let at = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let model = ActionModel::baseline();
    let traced = balanced.decide(&at, Some(&model), &[], BUDGET, 60, &[]);
    assert_eq!(traced.trace.unwrap().priced.unwrap().exploit, Some(0.25));
    let reference = Policy::reference(Params {
        exploit: 1.0,
        spread: 0.0,
    });
    assert_eq!(reference.exploit(), 1.0);
}

#[test]
fn a_draw_stays_within_the_spread_of_the_best_action() {
    // A spread of 2% of a 100-chip pot is a 2-chip band. Only the check and the call are inside
    // it; the small raise and the fold are not.
    let values = [
        (Action::Check, 100.0),
        (Action::Call, 99.0),
        (Action::RaiseTo(200), 96.0),
        (Action::Fold, 0.0),
    ];
    for seed in 0..500 {
        let action = pricer::draw_within_spread(&values, 100, 0.02, seed);
        let value = values
            .iter()
            .find(|(each, _)| *each == action)
            .expect("a priced action")
            .1;
        assert!(value >= 98.0 - 1e-9, "seed {seed}: {action:?} is {value}");
    }
    // A zero spread leaves only a best action; the check is the one.
    for seed in 0..100 {
        assert_eq!(
            pricer::draw_within_spread(&values, 100, 0.0, seed),
            Action::Check
        );
    }
}

#[test]
fn a_draw_is_deterministic_for_a_seed_and_can_vary_between_seeds() {
    let values = [
        (Action::Check, 10.0),
        (Action::Call, 9.0),
        (Action::RaiseTo(100), 8.0),
    ];
    let first = pricer::draw_within_spread(&values, 100, 0.5, 7);
    assert_eq!(first, pricer::draw_within_spread(&values, 100, 0.5, 7));
    let mut seen = [false; 3];
    for seed in 0..500 {
        let action = pricer::draw_within_spread(&values, 100, 0.5, seed);
        seen[match action {
            Action::Check => 0,
            Action::Call => 1,
            _ => 2,
        }] = true;
    }
    assert!(seen.iter().filter(|seen| **seen).count() >= 2, "{seen:?}");
}

#[test]
fn a_balanced_pricer_draws_and_keeps_the_draw_in_the_trace() {
    let spot = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let policy = Policy::new(PolicyKind::Balanced, &style::ANCHOR, params(0.5));
    let decision = policy.decide(&spot, None, &[], BUDGET, 60, &[]);
    let trace = decision.trace.as_ref().expect("the policy traces");
    assert_eq!(trace.branch, svanbot_strategy::policy::Branch::Priced);
    assert!(trace.resolved.is_none());
    let drawn = decision.drawn.clone().expect("a Balanced pricer mixes");
    assert!(drawn.taken < drawn.values.len());
    assert!((drawn.strategy.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    // The drawn action's value is inside the spread of the best.
    let best = drawn
        .values
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        drawn.values[drawn.taken] >= best - 0.5 * spot.pot as f64 - 1e-6,
        "{drawn:?}"
    );
    // The same seed repeats the decision.
    assert_eq!(decision, policy.decide(&spot, None, &[], BUDGET, 60, &[]));
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
fn a_balanced_policy_holds_reads_and_uses_only_the_population() {
    let board = "AhKs9d4c2s";
    let mut at = spot("7c7d", board, 600, 200, 2);
    let called = Act {
        street: Street::River,
        board: board.parse().unwrap(),
        facing: true,
        kind: ActKind::Call,
    };
    at.lines = vec![vec![called], vec![called]];
    at.seats = vec![1, 3];
    let personal = [Personal {
        seat: 3,
        model: calls_only_the_nuts(),
        class: Class::Fixed,
        trust: Some(0.62),
        read: None,
    }];
    let policy = Policy::new(PolicyKind::Balanced, &style::ANCHOR, params(0.05));
    let read = policy.decide(&at, None, &personal, BUDGET, 60, &[]);
    let held = policy.decide(&at, None, &[], BUDGET, 60, &[]);
    assert_eq!(read.action, held.action);
    let priced = read.trace.as_ref().unwrap().priced.as_ref().unwrap();
    // The read is kept out of the decision: the trace says no class and no trust.
    assert_eq!(priced.ranges[1].class, None);
    assert_eq!(priced.ranges[1].trust, None);
    assert_eq!(priced.ranges[1].masses, priced.ranges[0].masses);
}

#[test]
fn a_balanced_policy_does_not_use_the_profile() {
    // The station profile folds a weak hand facing a bet; the Balanced kind must decide without
    // it even when the style it was built with carries the active profile.
    use svanbot_strategy::profile::{OpponentStats, Rate, SeatedStats};

    let board = "AhKsQh";
    let mut at = spot("7c2d", board, 400, 200, 1);
    let called = Act {
        street: Street::Flop,
        board: board.parse().unwrap(),
        facing: true,
        kind: ActKind::Call,
    };
    at.lines = vec![vec![called]];
    at.seats = vec![1];
    let mut vpip = Rate::default();
    let mut pfr = Rate::default();
    let mut fold = Rate::default();
    for _ in 0..30 {
        vpip.observe(1, 1);
        pfr.observe(0, 1);
        fold.observe(0, 1);
    }
    let stats = [SeatedStats {
        seat: 1,
        stats: OpponentStats {
            vpip,
            pfr,
            fold_to_bet: fold,
            ..OpponentStats::default()
        },
    }];
    let balanced = Policy::new(PolicyKind::Balanced, &style::STANDARD, params(0.05));
    let with_stats = balanced.decide(&at, None, &[], BUDGET, 60, &stats);
    let without = balanced.decide(&at, None, &[], BUDGET, 60, &[]);
    assert_eq!(with_stats, without);
    // The style still carries its profile, and the same spot under Solving lets it fire: the
    // kind is what ignores it, not an empty style.
    assert!(!style::STANDARD.profile.is_empty());
    let solving = Policy::new(PolicyKind::Solving, &style::STANDARD, params(0.05));
    assert!(
        solving
            .decide(&at, None, &[], BUDGET, 60, &stats)
            .trace
            .is_none(),
        "the active profile did not fire; this test would not distinguish"
    );
}

#[test]
fn the_balanced_opening_boundary_is_a_chance_at_the_cut() {
    use svanbot_strategy::preflop::{self, Position};

    let position = Position {
        behind: 5,
        in_position: false,
    };
    let mut borderline = Vec::new();
    for class in 0..preflop::CLASSES {
        let chance = preflop::open_chance(class, position, 1.0);
        if chance > 0.0 && chance < 1.0 {
            borderline.push(class);
        }
        assert_eq!(
            chance > 0.5,
            preflop::opens_at(class, position, 1.0),
            "{class}"
        );
    }
    assert!(!borderline.is_empty(), "no class sits on the boundary");
}

#[test]
fn a_balanced_opening_spot_opens_a_borderline_class_with_its_chance() {
    use svanbot_strategy::preflop::{self, Position};

    let position = Position {
        behind: 5,
        in_position: false,
    };
    let class = (0..preflop::CLASSES)
        .find(|&class| {
            let chance = preflop::open_chance(class, position, 1.0);
            chance > 0.25 && chance < 0.75
        })
        .expect("a borderline class");
    let chance = preflop::open_chance(class, position, 1.0);
    let mut at = spot("2c2d", "", 30, 20, 5);
    at.hole = preflop::example(class);
    at.lines = vec![vec![]; 5];
    at.legal = Legal {
        call: Some(20),
        raise: Some(RaiseRange {
            min: 40,
            max: 2_000,
        }),
    };
    let policy = Policy::new(PolicyKind::Balanced, &style::ANCHOR, params(0.1));
    let mut opened = 0;
    for seed in 0..200 {
        let budget = Budget { samples: 640, seed };
        let action = policy.decide(&at, None, &[], budget, 60, &[]).action;
        if matches!(action, Action::RaiseTo(_)) {
            opened += 1;
        }
    }
    let observed = f64::from(opened) / 200.0;
    assert!(
        (observed - chance).abs() < 0.15,
        "chance {chance}, opened {observed}"
    );
    assert!(opened > 0 && opened < 200, "opened {opened}");
}
