//! The trace: the record of what made one decision, kept with its hand so the replayer can read
//! it rather than play the hand again.

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Legal, RaiseRange, Street};
use svanbot_core::spot::Spot;
use svanbot_strategy::action::{
    ActionModel, BUCKETS, Move, Seen, Situation, masses, range, ranked,
};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{Branch, decide_drawn};
use svanbot_strategy::pricer::Budget;

#[test]
fn a_range_reads_as_masses_over_the_strength_buckets() {
    // The range is 1,176 weighted combos; the trace keeps eight masses. The same eight buckets the
    // decision was made in, so the reader and the solver are looking at one thing.
    let model = ActionModel::baseline();
    let seen = [Seen {
        board: cards("QhJhTh"),
        situation: Situation {
            street: Street::Flop,
            facing: true,
        },
        mv: Move::Large,
    }];
    let read = range(&model, &seen, cards("AhKh"));
    let masses = masses(&read, &ranked(cards("QhJhTh")));
    assert_eq!(masses.len(), BUCKETS);
    assert!((masses.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    // A big bet on that board moves the villain's range off the weak buckets.
    assert!(masses[BUCKETS - 1] > masses[0]);
}

#[test]
fn a_summary_of_an_empty_range_is_empty_rather_than_a_division_by_zero() {
    let masses = masses(&[], &ranked(cards("QhJhTh")));
    assert_eq!(masses.len(), BUCKETS);
    assert!(masses.iter().all(|mass| mass.is_finite()));
    assert!(masses.iter().sum::<f64>().abs() < 1e-12);
}

#[test]
fn a_resolved_trace_carries_its_values_the_range_it_read_and_what_each_action_was() {
    let river = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let decision = decide_drawn(&river, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    assert_eq!(trace.branch, Branch::Resolved);
    let resolved = trace
        .resolved
        .as_ref()
        .expect("a re-solve keeps its values");
    assert_eq!(resolved.values.len(), resolved.strategy.len());
    // Every value has a name, and the taken index names one of them.
    assert_eq!(resolved.actions.len(), resolved.values.len());
    assert!(resolved.taken < resolved.actions.len());
    // Every opponent's range is summarised, and there is one opponent here.
    assert_eq!(resolved.ranges.len(), 1);
    assert_eq!(resolved.ranges[0].masses.len(), BUCKETS);
}

#[test]
fn a_trace_carries_the_cards_the_decision_was_made_with() {
    // The ticket's third item: the trace carries the hole cards and the board. A replayer that
    // reads it needs nothing from `hands.frames` to say what the bot held.
    let turn = spot("AhKh", "QhJhTh2c3d", 400, 200, 1);
    let decision = decide_drawn(&turn, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    assert_eq!(trace.hole, cards("AhKh"));
    assert_eq!(trace.board, cards("QhJhTh2c3d"));
}

#[test]
fn a_priced_trace_keeps_its_equity_and_the_price_it_compared_against() {
    // A call of 200 into a pot of 400: the price of the call is 200/(400+200), and the pricer
    // compares the equity against exactly that.
    let preflop = spot("AhAs", "", 400, 200, 1);
    let decision = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    assert_eq!(trace.branch, Branch::Priced);
    assert!(trace.resolved.is_none());
    let priced = trace
        .priced
        .as_ref()
        .expect("the pricer keeps its comparison");
    assert_eq!(priced.price, Some(200.0 / 600.0));
    assert!(priced.equity > 0.0 && priced.equity <= 1.0);
}

#[test]
fn a_trace_says_when_no_fitted_model_passed_the_gate() {
    // No model is no exploit: the trace says `None` rather than a budget of zero, which would
    // read as a fit that was allowed to move nothing.
    let preflop = spot("AhAs", "", 400, 200, 1);
    let decision = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    let priced = trace.priced.as_ref().expect("priced preflop");
    assert_eq!(priced.exploit, None);
}

#[test]
fn a_free_check_prices_nothing() {
    // No call to pay: the pricer's comparison has no price, and the trace says so rather than
    // writing a zero that would read as a free call.
    let mut spot = spot("AhAs", "", 400, 200, 1);
    spot.legal = Legal {
        call: None,
        raise: None,
    };
    let decision = decide_drawn(&spot, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    let priced = trace.priced.as_ref().expect("priced preflop");
    assert_eq!(priced.price, None);
}

#[test]
fn an_opponent_with_no_line_reads_at_the_uniform_range() {
    // A villain who has not acted yet is not a villain with no cards. The trace says what the
    // pricer and the re-solve both assume: every pair equally likely, one eighth a bucket.
    let preflop = spot("AhAs", "", 400, 200, 1);
    let decision = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    let priced = trace.priced.as_ref().expect("priced preflop");
    let masses = &priced.ranges[0].masses;
    assert_eq!(masses.len(), BUCKETS);
    for mass in masses {
        assert!((mass - 1.0 / BUCKETS as f64).abs() < 1e-9, "{masses:?}");
    }
}

#[test]
fn every_opponent_gets_his_own_summary() {
    // Two villains are two ranges, and averaging them into one would describe a player who was
    // never at the table.
    let mut multiway = spot("7c2d", "AhKsQh", 400, 200, 2);
    multiway.lines = vec![
        vec![act(Street::Flop, true, 400, 200)],
        vec![act(Street::Flop, true, 400, 200)],
    ];
    multiway.seats = vec![4, 1];
    let decision = decide_drawn(&multiway, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    let priced = trace.priced.as_ref().expect("multiway is priced");
    assert_eq!(priced.ranges.len(), 2);
    // Each range names the seat it read, not a position in the list: a reader matches "seat 4"
    // to the table, and "opponent 1" to nothing on it.
    let seats: Vec<Option<usize>> = priced.ranges.iter().map(|range| range.seat).collect();
    assert_eq!(seats, vec![Some(4), Some(1)]);
}

#[test]
fn a_seat_is_unknown_rather_than_guessed_when_the_spot_carries_none() {
    // A spot a caller built without seats (a sim, or a hand-made test) must not invent one from
    // the ordinal: the trace says the seat is unknown, which reads as unknown, not as seat 1.
    let preflop = spot("AhAs", "", 400, 200, 1);
    assert!(preflop.seats.is_empty());
    let decision = decide_drawn(&preflop, None, &Params::DEFAULT, BUDGET, 60);
    let trace = decision.trace.as_ref().expect("the policy traces");
    let priced = trace.priced.as_ref().expect("priced preflop");
    assert_eq!(priced.ranges[0].seat, None);
}

const BUDGET: Budget = Budget {
    samples: 3_200,
    seed: 5,
};

fn cards(text: &str) -> CardSet {
    text.parse().unwrap()
}

fn act(street: Street, facing: bool, added: u64, pot: u64) -> svanbot_core::spot::Act {
    svanbot_core::spot::Act {
        street,
        board: cards("AhKsQh"),
        facing,
        kind: svanbot_core::spot::ActKind::Bet { added, pot },
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
        seats: vec![],
        lines: vec![],
    }
}
