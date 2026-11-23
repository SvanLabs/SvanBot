//! Per-opponent reads: a name's own class, change point, evidence and earned share.

use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_learn::reads::{Reads, Witness};
use svanbot_strategy::action::{ActionModel, Move, Seen, Situation};
use svanbot_strategy::read::Class;

fn open() -> Situation {
    Situation {
        street: Street::Preflop,
        facing: false,
    }
}

fn facing() -> Situation {
    Situation {
        street: Street::Preflop,
        facing: true,
    }
}

fn seen(mv: Move) -> Seen {
    Seen {
        board: CardSet::EMPTY,
        situation: open(),
        mv,
    }
}

fn hand(name: &str, aggressive: bool, vs_us: Option<bool>, vs_others: Option<bool>) -> Witness {
    Witness {
        name: name.into(),
        actions: vec![seen(if aggressive { Move::Large } else { Move::Check })],
        aggressive,
        vs_us,
        vs_others,
    }
}

fn fixed(name: &str, hands: usize) -> Reads {
    let mut reads = Reads::default();
    for _ in 0..hands {
        reads.observe(hand(name, false, None, None));
    }
    reads
}

#[test]
fn a_thin_opponent_is_unknown_and_earns_nothing() {
    let reads = fixed("alpha_bot", 10);
    let alpha = reads.get("alpha_bot").unwrap();
    assert_eq!(alpha.class(), Class::Unknown);
    assert_eq!(alpha.change(), None);
    assert_eq!(alpha.hands(), 10);
    assert_eq!(alpha.earned(), 0.0);
    assert_eq!(alpha.trust(open()), 0.0);
}

#[test]
fn a_steady_opponent_is_fixed_and_earns_most_of_the_budget() {
    let reads = fixed("alpha_bot", 60);
    let alpha = reads.get("alpha_bot").unwrap();
    assert_eq!(alpha.class(), Class::Fixed);
    assert_eq!(alpha.change(), None);
    assert_eq!(alpha.hands(), 60);
    assert_eq!(alpha.actions(), 60);
    // 60 actions at the half-trust point of 30: two thirds of the class's budget.
    assert!((alpha.earned() - 2.0 / 3.0).abs() < 1e-9);
    assert!((alpha.trust(open()) - 2.0 / 3.0).abs() < 1e-9);
    assert_eq!(alpha.trust(facing()), 0.0);
}

#[test]
fn a_switching_opponent_s_read_restarts_at_the_change() {
    let mut reads = Reads::default();
    for _ in 0..30 {
        reads.observe(hand("alpha_bot", false, None, None));
    }
    for _ in 0..30 {
        reads.observe(hand("alpha_bot", true, None, None));
    }
    let alpha = reads.get("alpha_bot").unwrap();
    assert_eq!(alpha.class(), Class::Switching);
    assert_eq!(alpha.change(), Some(30));
    assert_eq!(alpha.hands(), 60);
    // Only the hands after the change feed the read.
    assert_eq!(alpha.actions(), 30);
    assert_eq!(alpha.read().n(open()), 30);
    assert_eq!(alpha.read().frequency(open(), Move::Check, 0.0, 0.0), 0.0);
    assert!((alpha.earned() - 0.5).abs() < 1e-9);
}

#[test]
fn an_opponent_who_learns_from_us_is_damped() {
    let mut reads = Reads::default();
    for number in 0..60 {
        // The gap against others is there from the start and widens against us.
        let vs_us = number >= 30 || number % 2 == 0;
        reads.observe(hand("alpha_bot", false, Some(vs_us), Some(false)));
    }
    let alpha = reads.get("alpha_bot").unwrap();
    assert_eq!(alpha.class(), Class::Learning);
    assert_eq!(alpha.change(), None);
    assert_eq!(alpha.actions(), 60);
    // A quarter of the budget, at two thirds of the evidence: one sixth.
    assert!((alpha.earned() - 1.0 / 6.0).abs() < 1e-9);
}

#[test]
fn each_name_has_its_own_read_in_alphabetical_order() {
    let mut reads = fixed("beta_bot", 60);
    for _ in 0..10 {
        reads.observe(hand("alpha_bot", true, None, None));
    }
    let names: Vec<&str> = reads.opponents().map(|opponent| opponent.name()).collect();
    assert_eq!(names, ["alpha_bot", "beta_bot"]);
    assert_eq!(reads.get("alpha_bot").unwrap().class(), Class::Unknown);
    assert_eq!(reads.get("beta_bot").unwrap().class(), Class::Fixed);
    assert!(reads.get("gamma_bot").is_none());
}

#[test]
fn a_bulk_load_reads_the_same_as_hand_by_hand() {
    // A switch at 30 for alpha, and a steady beta: the change point and the restarted read too.
    let hands: Vec<Witness> = (0..60)
        .flat_map(|number| {
            [
                hand("alpha_bot", number >= 30, None, None),
                hand("beta_bot", false, Some(number % 3 == 0), Some(false)),
            ]
        })
        .collect();
    let mut one_by_one = Reads::default();
    for witness in hands.clone() {
        one_by_one.observe(witness);
    }
    let bulk = Reads::from_witnesses(hands);
    assert_eq!(bulk, one_by_one);
    assert_eq!(bulk.get("alpha_bot").unwrap().change(), Some(30));
}

#[test]
fn each_seated_opponent_goes_to_the_decision_with_their_class_and_trust() {
    // beta checks every hand: a fixed read that tilts their model toward checking.
    let reads = fixed("Beta_Bot", 60);
    let population = ActionModel::baseline();
    let seated = [(1, "beta_bot"), (4, "gamma_bot")];
    let personal = reads.personal(&seated, &population, 0.5);
    assert_eq!(personal.len(), 2);
    let (beta, gamma) = (&personal[0], &personal[1]);
    assert_eq!((beta.seat, beta.class), (1, Class::Fixed));
    assert!((beta.trust - 2.0 / 3.0).abs() < 1e-9, "{}", beta.trust);
    let check = |model: &ActionModel| model.marginal(open(), Move::Check);
    assert!(check(&beta.model) > check(&population));
    // Never seen: unknown, trusted with nothing, read by the population.
    assert_eq!(
        (gamma.seat, gamma.class, gamma.trust),
        (4, Class::Unknown, 0.0)
    );
    assert_eq!(gamma.model, population);
    // No exploit budget leaves every model the population's.
    for read in reads.personal(&seated, &population, 0.0) {
        assert_eq!(read.model, population);
    }
}
