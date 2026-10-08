#[test]
fn fold_logit_shift_moves_folds_monotonically_and_zero_is_identity() {
    for fold in [0.0, 0.05, 0.3, 0.6, 0.99] {
        assert_eq!(shift_fold_logit(fold, 0.0), fold);
    }
    assert_eq!(shift_fold_logit(0.0, 1.0), 0.0, "a responder who never folds stays at zero");
    let base = 0.30;
    let river = shift_fold_logit(base, -0.74);
    assert!((river - 0.1698).abs() < 1e-3, "river -0.74 logit on 30%: {river}");
    assert!(shift_fold_logit(base, 0.21) > base);
    assert!(shift_fold_logit(0.98, 3.0) <= 0.99);
}

use super::*;
use sv10_engine::engine::{Action, ActionKind, ActionRecord, Hand};
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

fn prices(sit: &Situation) -> Vec<(f64, ResponderPrice)> {
    let models = ModelStore::default();
    let params = Params { fold_scale: 0.7, preflop_fold_scale: 0.9, passive_fold_bonus: 0.03, ..Params::default() };
    let ranges = estimate_ranges(sit, &models, &params.range);
    let (responders, _) = Responder::for_table(sit, &models, &ranges);
    let to = sit.min_raise_to.expect("raise available").min(sit.max_raise_to.expect("raise maximum"));
    let pricing = ResponsePricing::new(sit, &models, &params, None, false);
    responders.iter().zip(pricing.for_raise_to(&responders, to)).map(|(responder, price)| (responder.range.total(), price)).collect()
}

fn assert_invariants(sit: &Situation) {
    for (source_mass, price) in prices(sit) {
        assert!(price.fold_prob.is_finite() && (0.0..=1.0).contains(&price.fold_prob));
        assert!(price.continue_prob.is_finite() && (0.0..=1.0).contains(&price.continue_prob));
        assert!(price.raise_given_continue.is_finite() && (0.0..=1.0).contains(&price.raise_given_continue));
        assert!((price.fold_prob + price.continue_prob - 1.0).abs() < 1e-12);
        let expected_mass = source_mass * price.continue_prob;
        assert!((price.continue_range.total() - expected_mass).abs() < 2e-4, "continuing range mass disagrees with probability");
    }
}

#[test]
fn short_stacks_open_to_their_own_size_and_the_jam_threshold_is_a_knob() {
    // 0171: an unopened UTG spot at 25 bb (short) and 100 bb (deep), blinds 10/20.
    let names = (0..4).map(|seat| format!("p{seat}")).collect::<Vec<_>>();
    let targets = |stack: i64, params: &Params| {
        let hand = Hand::new(&[stack; 4], 0, 10, 20, &mut SmallRng::seed_from_u64(9));
        let sit = Situation::from_hand(&hand, hand.actor().unwrap(), &names);
        raise_targets(&sit, params, false, 0.5)
    };
    let defaults = Params::default();
    assert!(targets(500, &defaults).contains(&50) && targets(2_000, &defaults).contains(&50), "defaults open 2.5 bb at every depth");
    assert!(targets(500, &defaults).contains(&500), "and may jam at 25 bb, as before");
    let short = Params { short_open_bb: 2.0, ..Params::default() };
    assert!(targets(500, &short).contains(&40) && !targets(500, &short).contains(&50), "{:?}", targets(500, &short));
    assert!(targets(2_000, &short).contains(&50), "deep stacks keep open_bb");
    let tight = Params { preflop_jam_bb: 20.0, ..Params::default() };
    assert!(!targets(500, &tight).contains(&500), "25 bb is above a 20 bb jam threshold");
    assert!(targets(500, &tight).contains(&50), "above the threshold the stack is not short: open_bb");
}

#[test]
fn responder_price_invariants_hold_preflop_flop_and_multiway() {
    let mut rng = SmallRng::seed_from_u64(45);
    let preflop = Hand::new(&[2_000; 4], 0, 10, 20, &mut rng);
    let names = (0..4).map(|seat| format!("p{seat}")).collect::<Vec<_>>();
    assert_invariants(&Situation::from_hand(&preflop, preflop.actor().unwrap(), &names));

    let mut flop = Hand::new(&[2_000; 3], 0, 10, 20, &mut rng);
    flop.apply(Action::Call).unwrap();
    flop.apply(Action::Call).unwrap();
    flop.apply(Action::Check).unwrap();
    let names = (0..3).map(|seat| format!("p{seat}")).collect::<Vec<_>>();
    assert_eq!(flop.street, Street::Flop);
    assert_invariants(&Situation::from_hand(&flop, flop.actor().unwrap(), &names));
}

#[test]
fn a_caller_of_an_all_in_is_not_priced_to_fold_to_a_squeeze() {
    // A predicted 99% fold is no edge against a player who just called an all-in: SuraGunnar
    // jammed 6-3o for 490 bb into one and was called (stored hands: they fold 5 of 18 times).
    let mut rng = SmallRng::seed_from_u64(5);
    let mut hand = Hand::new(&[10_000, 1_600, 24_000], 0, 10, 20, &mut rng);
    hand.apply(Action::RaiseTo(760)).unwrap(); // hero (button) opens
    hand.apply(Action::AllIn).unwrap(); // small blind shoves
    hand.apply(Action::Call).unwrap(); // big blind calls the shove
    assert_eq!(hand.actor(), Some(0));
    let names: Vec<String> = ["hero", "shover", "caller"].iter().map(|s| s.to_string()).collect();
    let sit = Situation::from_hand(&hand, 0, &names);
    // A profile that "always folds to a 4-bet" on a tiny sample, the case that fooled the model.
    let mut models = ModelStore::default();
    let st = sv10_model::model::PlayerStats {
        hands: 5_000.0,
        fold_to_4bet: sv10_model::model::Counter { opp: 10.0, hit: 10.0 },
        ..Default::default()
    };
    models.players.insert("caller".into(), st);
    let params = Params::default();
    let ranges = estimate_ranges(&sit, &models, &params.range);
    let (responders, _) = Responder::for_table(&sit, &models, &ranges);
    let pricing = ResponsePricing::new(&sit, &models, &params, None, false);
    let max_to = sit.max_raise_to.unwrap();
    for (responder, price) in responders.iter().zip(pricing.for_raise_to(&responders, max_to)) {
        if responder.seat == 2 {
            assert!(responder.called_all_in, "the big blind called an all-in");
            assert!(price.fold_prob <= CALLED_ALL_IN_FOLD_CAP + 1e-12, "fold {}", price.fold_prob);
        }
    }
}

#[test]
fn all_in_responders_cannot_fold_or_raise() {
    let sit = sv10_engine::situation::fixtures::river_jam_with_all_ins();
    let models = ModelStore::default();
    let params = Params::default();
    let ranges = estimate_ranges(&sit, &models, &params.range);
    let (responders, _) = Responder::for_table(&sit, &models, &ranges);
    let pricing = ResponsePricing::new(&sit, &models, &params, None, false);
    for (responder, price) in responders.iter().zip(pricing.for_raise_to(&responders, 20)) {
        if responder.stack_total <= responder.bet {
            assert_eq!((price.fold_prob, price.continue_prob, price.raise_given_continue), (0.0, 1.0, 0.0));
        }
    }
}

#[test]
fn prior_street_aggression_does_not_trigger_current_street_fold_cap() {
    let mut sit = sv10_engine::situation::fixtures::river_jam_with_all_ins();
    sit.history.clear();
    let baseline = sit.clone();
    sit.history.push(ActionRecord {
        seat: 0,
        street: Street::Flop,
        kind: ActionKind::Raise,
        to: 100,
        pot_before: 100,
        to_call_before: 0,
        bet_before: 0,
        full_raise: true,
        think_ms: None,
        street_open: false,
    });
    let mut range = Range::default();
    range.normalize();
    let models = ModelStore::default();
    let profile = models.profile("villain");
    let responder = || Responder {
        seat: 0,
        stack_total: 10_337,
        bet: 0,
        profile: profile.clone(),
        range: &range,
        acted_this_street: false,
        called_all_in: false,
    };
    let params = Params::default();
    let before = ResponsePricing::new(&baseline, &models, &params, None, false).for_raise_to(&[responder()], 1_000);
    let after = ResponsePricing::new(&sit, &models, &params, None, false).for_raise_to(&[responder()], 1_000);
    assert!((before[0].fold_prob - after[0].fold_prob).abs() < 1e-12);
}

#[test]
fn short_stack_response_vectors_match_observed_training_decisions() {
    use sv10_model::features::{ResponseFeatureSet, features_for, mask, named_samples_from_hand_for};
    use sv10_model::model::HandSummary;
    for opponent_stack in [100, 4_000] {
        let mut hand = Hand::new(&[4_000, opponent_stack], 0, 10, 20, &mut SmallRng::seed_from_u64(908));
        let hero = hand.actor().unwrap();
        let names = vec!["hero".to_string(), "villain".to_string()];
        assert_eq!(hero, 0);
        let sit = Situation::from_hand(&hand, hero, &names);
        let models = ModelStore::default();
        let params = Params::default();
        let ranges = estimate_ranges(&sit, &models, &params.range);
        let (responders, _) = Responder::for_table(&sit, &models, &ranges);
        let pricing = ResponsePricing::new(&sit, &models, &params, None, false);
        let context = pricing.response_context(&responders[0], 1, 1_000);
        hand.apply(Action::RaiseTo(1_000)).unwrap();
        hand.apply(Action::Call).unwrap();
        let summary = HandSummary {
            players: names.into_iter().enumerate().collect(),
            button: hand.button,
            bb: 20,
            history: hand.history.clone(),
            board: hand.board.clone(),
            shown: vec![],
            stacks: hand.seats.iter().enumerate().map(|(seat, s)| (seat, s.start_stack)).collect(),
        };
        for inputs in [37, 38, 39] {
            let layout = ResponseFeatureSet::for_inputs(inputs).unwrap();
            let samples = named_samples_from_hand_for(&summary, &models, &[], layout);
            let (_, trained, _) = samples.iter().find(|(name, _, _)| name == "villain").unwrap();
            assert_eq!(features_for(&context, layout), trained.x, "{inputs} inputs, stack {opponent_stack}");
            assert_eq!(mask(&context), trained.mask, "response classes agree");
        }
        assert_eq!(context.to_call, 980, "raw amount faced is independent of payable chips");
        assert_eq!(context.stack_behind, opponent_stack - 20);
    }
}
