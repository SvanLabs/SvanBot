use svanbot_core::rules::Blinds;
use svanbot_learn::paired::Match;
use svanbot_learn::play::{player, profile_stats, reading_model, rival_duel, rival_ring_at};
use svanbot_learn::reads::Reads;
use svanbot_strategy::{
    kind::PolicyKind, params::Params, policy::Policy, profile::Profile, style::Style,
};

const GAME: Match = Match {
    blinds: Blinds { small: 10, big: 20 },
    stack: 2000,
    seed: 7,
};

#[test]
fn a_policy_playing_itself_round_a_table_breaks_even() {
    let play = player(Policy::reference(Params::DEFAULT), Some(2));
    for deal in 0..2 {
        assert!(rival_ring_at(&GAME, 3, deal, &play, &play).abs() < 1e-9);
    }
}

#[test]
fn a_heads_up_stablemate_deal_plays_the_same_twice() {
    let play = player(Policy::reference(Params::DEFAULT), Some(2));
    let first = rival_duel(&GAME, 0, &play, &play);
    assert!(first.is_finite());
    assert_eq!(
        first.to_bits(),
        rival_duel(&GAME, 0, &play, &play).to_bits()
    );
}

#[test]
fn the_paired_decider_supplies_the_fitted_model_as_the_population_prior() {
    use svanbot_learn::arena::BoutReader;
    use svanbot_strategy::{action::ActionModel, policy::Personal};

    fn take(_: &Reads, _: &[(usize, &str)], population: &ActionModel, _: f64) -> Vec<Personal> {
        assert_eq!(*population, ActionModel::uniform());
        Vec::new()
    }

    let policy = Policy::reference(Params::DEFAULT);
    let fitted = reading_model(policy, Some(2), 6, take, Some(ActionModel::uniform()));
    let reader = BoutReader::new(fitted);
    let reference = player(policy, Some(2));
    assert!(rival_ring_at(&GAME, 6, 1, &reader, &reference).is_finite());
}

#[test]
fn each_paired_decider_supplies_its_own_model_as_the_population_prior() {
    use std::cell::RefCell;
    use svanbot_learn::arena::BoutReader;
    use svanbot_strategy::{action::ActionModel, policy::Personal};

    thread_local! {
        static SEEN: RefCell<Vec<ActionModel>> = const { RefCell::new(Vec::new()) };
    }
    fn record(_: &Reads, _: &[(usize, &str)], population: &ActionModel, _: f64) -> Vec<Personal> {
        SEEN.with(|seen| seen.borrow_mut().push(population.clone()));
        Vec::new()
    }

    let policy = Policy::reference(Params::DEFAULT);
    let reference = player(policy, Some(2));
    for model in [ActionModel::uniform(), ActionModel::baseline()] {
        SEEN.with(|seen| seen.borrow_mut().clear());
        let decider = reading_model(policy, Some(2), 6, record, Some(model.clone()));
        let reader = BoutReader::new(decider);
        assert!(rival_ring_at(&GAME, 6, 1, &reader, &reference).is_finite());
        let seen = SEEN.with(|seen| seen.borrow().clone());
        assert!(!seen.is_empty());
        assert!(seen.iter().all(|seen| *seen == model));
    }
}

#[test]
fn a_style_with_rules_collects_stats_per_seat() {
    let policy = Policy::reference(Params::DEFAULT);
    let stats = profile_stats(&policy, &Reads::default(), &[(1, "x"), (3, "y")]);
    assert_eq!(
        stats.iter().map(|stats| stats.seat).collect::<Vec<_>>(),
        [1, 3]
    );
}

#[test]
fn a_style_with_no_rules_collects_none() {
    static BARE: Style = Style {
        name: "bare",
        open_scale: 1.0,
        mechanics: svanbot_strategy::style::NEUTRAL,
        profile: Profile::new(&[]),
        bounds: &[],
    };
    let policy = Policy::new(PolicyKind::Solving, &BARE, Params::DEFAULT);
    assert!(profile_stats(&policy, &Reads::default(), &[(1, "x")]).is_empty());
}

#[test]
fn a_resolved_decision_notes_its_draw_and_a_priced_one_notes_none() {
    use svanbot_core::{
        rules::{Legal, RaiseRange},
        spot::Spot,
    };
    use svanbot_learn::{
        arena::BoutReader,
        paired::{Challenger, Draws},
        play::reading,
    };

    let draws = Draws::default();
    let policy = Policy::reference(Params::DEFAULT);
    let decide = reading(policy, Some(20), 2, Reads::deciding, Some(draws.clone()));
    let reader = BoutReader::new(decide).with_draws(draws);
    let mut spot = Spot {
        hole: "AhKh".parse().unwrap(),
        board: "QhJhTh".parse().unwrap(),
        pot: 400,
        legal: Legal {
            call: Some(200),
            raise: Some(RaiseRange {
                min: 400,
                max: 2000,
            }),
        },
        opponents: 1,
        stack: 2000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        players_behind: None,
        lines: vec![],
        seats: vec![],
    };
    reader.decide(&spot, 7);
    let mut reported = reader.take_decided();
    assert_eq!(reported.len(), 1);
    assert!((reported.pop().unwrap().strategy.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    spot.board = "".parse().unwrap();
    assert!(reader.take_decided().is_empty());
    reader.decide(&spot, 7);
    assert!(reader.take_decided().is_empty());
}

#[test]
fn the_playbook_collects_stats_even_with_no_profile_rules() {
    static BARE: Style = Style {
        name: "bare",
        open_scale: 1.0,
        mechanics: svanbot_strategy::style::NEUTRAL,
        profile: Profile::new(&[]),
        bounds: &[],
    };
    let policy = Policy::new(PolicyKind::Exploiting, &BARE, Params::DEFAULT);
    let stats = profile_stats(&policy, &Reads::default(), &[(1, "x")]);
    assert_eq!(stats.len(), 1);
}
