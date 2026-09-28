//! Property tests: invariants checked over many random, seeded cases rather than fixed examples.
//! A failure prints its case seed; rerun one case with `SV10_PROP_SEED=<seed>`. The case count
//! defaults to a quick run and scales with `SV10_PROP_CASES` (the anti-regression gate's `deep`
//! mode raises it).
//!
//! - Engine: any sequence of legal actions conserves chips, never drives a stack negative, ends
//!   the hand, and settles to nets that sum to zero.
//! - Policy: from any reachable state, the decision is one of the engine's legal actions (raise
//!   sizes inside the legal bounds) and every candidate EV and probability is finite.

use sv10_core::engine::{Action, Hand, Legal};
use sv10_core::model::ModelStore;
use sv10_core::policy::{Params, decide};
use sv10_core::situation::Situation;
use sv10_rng::rngs::SmallRng;
use sv10_rng::{RngExt, SeedableRng};

fn cases(default: u64) -> u64 {
    std::env::var("SV10_PROP_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Run `f` once per case with its own seeded RNG, or only `SV10_PROP_SEED` when set.
fn for_cases(default: u64, mut f: impl FnMut(u64, &mut SmallRng)) {
    let seeds: Vec<u64> = match std::env::var("SV10_PROP_SEED").ok().and_then(|v| v.parse().ok()) {
        Some(s) => vec![s],
        None => (0..cases(default)).map(|i| 0x5eed_0000 + i).collect(),
    };
    for seed in seeds {
        let mut rng = SmallRng::seed_from_u64(seed);
        f(seed, &mut rng);
    }
}

/// A random table: 2–6 seats, stacks from under a big blind to deep, random button.
fn random_hand(rng: &mut SmallRng) -> Hand {
    let n = rng.random_range(2..=6usize);
    let stacks: Vec<i64> = (0..n)
        .map(|_| match rng.random_range(0..10) {
            0 => rng.random_range(1..=30),
            1..=6 => rng.random_range(200..=5_000),
            _ => rng.random_range(5_000..=200_000),
        })
        .collect();
    let button = rng.random_range(0..n);
    Hand::new(&stacks, button, 10, 20, rng)
}

/// A random legal action for `legal`.
fn random_action(legal: &Legal, rng: &mut SmallRng) -> Action {
    let mut options = vec![if legal.can_check { Action::Check } else { Action::Fold }];
    if !legal.can_check {
        options.push(Action::Call);
    }
    if let (Some(lo), Some(hi)) = (legal.min_raise_to, legal.max_raise_to) {
        options.push(Action::RaiseTo(if lo >= hi { hi } else { rng.random_range(lo..=hi) }));
        options.push(Action::AllIn);
    }
    options[rng.random_range(0..options.len())]
}

fn total_chips(h: &Hand) -> i64 {
    h.seats.iter().map(|s| s.stack).sum::<i64>() + h.pot()
}

fn assert_situation_matches_hand(h: &Hand, seed: u64, history: &[String]) {
    let actor = h.actor().expect("unfinished hand has an actor");
    let names: Vec<String> = (0..h.seats.len()).map(|i| format!("p{i}")).collect();
    let legal = h.legal();
    let sit = Situation::from_hand(h, actor, &names);
    assert_eq!(sit.hero_seat, actor, "seed {seed}, history {history:?}");
    assert_eq!(sit.pot, h.pot(), "seed {seed}, history {history:?}");
    assert_eq!(sit.current_bet(), h.current_bet(), "seed {seed}, history {history:?}");
    assert_eq!(sit.call_amount, legal.call_amount, "seed {seed}, history {history:?}");
    assert_eq!(sit.can_check, legal.can_check, "seed {seed}, history {history:?}");
    assert_eq!(sit.min_raise_to, legal.min_raise_to, "seed {seed}, history {history:?}");
    assert_eq!(sit.max_raise_to, legal.max_raise_to, "seed {seed}, history {history:?}");
    assert_eq!(sit.board, h.board, "seed {seed}, history {history:?}");
    assert_eq!(sit.street, h.street, "seed {seed}, history {history:?}");
    assert_eq!(sit.button, h.button, "seed {seed}, history {history:?}");
    for (actual, copied) in h.seats.iter().zip(&sit.players) {
        assert_eq!(
            (actual.stack, actual.bet, actual.folded),
            (copied.stack, copied.bet, copied.folded),
            "seed {seed}, history {history:?}"
        );
    }
}

fn assert_every_advertised_action_is_accepted(h: &Hand, seed: u64, history: &[String]) {
    let legal = h.legal();
    let mut actions = vec![Action::Fold, if legal.can_check { Action::Check } else { Action::Call }];
    if let (Some(lo), Some(hi)) = (legal.min_raise_to, legal.max_raise_to) {
        actions.push(Action::RaiseTo(lo));
        actions.push(Action::RaiseTo(hi));
        actions.push(Action::AllIn);
    }
    for action in actions {
        let mut probe = h.clone();
        probe.apply(action).unwrap_or_else(|error| panic!("seed {seed}: advertised {action:?} rejected: {error}; history {history:?}"));
    }
}

#[test]
fn legal_play_conserves_chips_and_settles_to_zero_sum() {
    for_cases(3_000, |seed, rng| {
        let mut h = random_hand(rng);
        let start: i64 = h.seats.iter().map(|s| s.start_stack).sum();
        let mut history = Vec::new();
        if !h.is_finished() {
            assert_eq!(total_chips(&h), start, "seed {seed}: blinds changed the chip total; history {history:?}");
        }
        let mut steps = 0;
        while !h.is_finished() {
            steps += 1;
            assert!(steps <= 400, "seed {seed}: hand did not finish; history {history:?}");
            assert_situation_matches_hand(&h, seed, &history);
            assert_every_advertised_action_is_accepted(&h, seed, &history);
            let action = random_action(&h.legal(), rng);
            h.apply(action).unwrap_or_else(|e| panic!("seed {seed}: legal {action:?} rejected: {e}; history {history:?}"));
            history.push(format!("{:?}:{action:?}", h.history.last().map(|record| record.street)));
            assert!(
                h.seats.iter().all(|s| s.stack >= 0 && s.invested >= 0),
                "seed {seed}: negative stack/contribution after {action:?}; history {history:?}"
            );
            if !h.is_finished() {
                assert_eq!(total_chips(&h), start, "seed {seed}: chips not conserved after {action:?}; history {history:?}");
            }
        }
        // Settled: payouts are back in the stacks.
        assert_eq!(
            h.seats.iter().map(|s| s.stack).sum::<i64>(),
            start,
            "seed {seed}: chips created or lost at settlement; history {history:?}"
        );
        let net = h.net();
        assert_eq!(net.iter().sum::<i64>(), 0, "seed {seed}: nets {net:?}; history {history:?}");
        assert_eq!(h.payouts().iter().sum::<i64>(), h.seats.iter().map(|s| s.invested).sum::<i64>(), "seed {seed}; history {history:?}");
        for (s, n) in h.seats.iter().zip(&net) {
            assert!(*n >= -s.start_stack, "seed {seed}: lost more than the stack; history {history:?}");
        }
    });
}

#[test]
fn decisions_are_legal_and_finite_from_any_reachable_state() {
    let models = ModelStore::default();
    let params = Params { samples: 150, ..Default::default() };
    for_cases(150, |seed, rng| {
        let mut h = random_hand(rng);
        // Walk a random number of legal actions in, then ask the policy for the actor.
        let walk = rng.random_range(0..12);
        for _ in 0..walk {
            if h.is_finished() {
                break;
            }
            let a = random_action(&h.legal(), rng);
            h.apply(a).unwrap();
        }
        let Some(actor) = h.actor() else { return };
        let names: Vec<String> = (0..h.seats.len()).map(|i| format!("p{i}")).collect();
        let sit = Situation::from_hand(&h, actor, &names);
        let legal = h.legal();
        let d = decide(&sit, &models, &params, rng);
        match d.action {
            Action::Check => assert!(legal.can_check, "seed {seed}: check not legal"),
            Action::Fold => {}
            Action::Call => assert!(!legal.can_check, "seed {seed}: call with nothing to call"),
            Action::RaiseTo(t) => {
                let (lo, hi) = (legal.min_raise_to, legal.max_raise_to);
                assert!(lo.is_some() && hi.is_some(), "seed {seed}: raise while raising is closed");
                assert!(t >= lo.unwrap().min(hi.unwrap()) && t <= hi.unwrap(), "seed {seed}: raise to {t} outside {lo:?}..{hi:?}");
            }
            Action::AllIn => assert!(legal.max_raise_to.is_some() || !legal.can_check, "seed {seed}: all-in not offered"),
        }
        // #424: a spot the draw cannot measure is refused rather than priced on 0.0, so a measured
        // decision's equity is a share and a refused one carries none.
        assert!(d.equity.is_none_or(|e| (0.0..=1.0).contains(&e)), "seed {seed}: equity {:?}", d.equity);
        for c in &d.candidates {
            assert!(c.ev.is_finite() && c.fold_prob.is_finite() && c.equity_called.is_finite(), "seed {seed}: {c:?}");
        }
        // The engine accepts what the policy chose.
        let mut after = h.clone();
        after.apply(d.action).unwrap_or_else(|e| panic!("seed {seed}: engine rejected {:?}: {e}", d.action));
    });
}

/// #424: a spot whose equity cannot be measured is refused rather than priced on `0.0`, which reads
/// as "hero never wins". The refusal is the safe action — check where the rules allow it, fold
/// otherwise — with no equity and a reason that says why. A zero-sample budget asks for no
/// measurement at all, which reaches the refusal without a table whose ranges collide on every deal
/// (that case is pinned in the equity crate's own tests).
#[test]
fn a_decision_with_no_equity_measurement_takes_the_safe_action() {
    let models = ModelStore::default();
    let params = Params { samples: 0, ..Default::default() };
    for_cases(30, |seed, rng| {
        let mut h = random_hand(rng);
        for _ in 0..rng.random_range(0..12) {
            if h.is_finished() {
                break;
            }
            let a = random_action(&h.legal(), rng);
            h.apply(a).unwrap();
        }
        let Some(actor) = h.actor() else { return };
        let names: Vec<String> = (0..h.seats.len()).map(|i| format!("p{i}")).collect();
        let sit = Situation::from_hand(&h, actor, &names);
        let legal = h.legal();
        let d = decide(&sit, &models, &params, rng);
        assert_eq!(d.equity, None, "seed {seed}: a draw asked for no samples still answered");
        let safe = if legal.can_check { Action::Check } else { Action::Fold };
        assert_eq!(d.action, safe, "seed {seed}: a refused decision did not take the safe action");
        assert!(d.reason.contains("no equity measurement"), "seed {seed}: reason is {:?}", d.reason);
        assert_eq!(d.chosen.action, d.action_name, "seed {seed}: the chosen candidate is not the action sent");
        // The refusal is still a decision the engine accepts.
        let mut after = h.clone();
        after.apply(d.action).unwrap_or_else(|e| panic!("seed {seed}: engine rejected {:?}: {e}", d.action));
    });
}
