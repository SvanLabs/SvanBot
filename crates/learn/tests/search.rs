//! The search (V11-49): what the learner sims next round the champion, unattended, and when a
//! challenger has earned a proposal.

use svanbot_learn::paired::Verdict;
use svanbot_learn::search::{Entry, Next, Stage, Step, next};
use svanbot_strategy::params::Params;

const CHAMPION: Params = Params {
    exploit: 0.5,
    spread: 0.0,
    ..Params::DEFAULT
};

fn at(exploit: f64) -> Params {
    Params {
        exploit,
        spread: 0.0,
        ..Params::DEFAULT
    }
}

/// The sim `next` asks for, or a panic naming what it asked for instead.
fn sim(found: Next) -> Step {
    match found {
        Next::Sim(step) => step,
        other => panic!("expected a sim, got {other:?}"),
    }
}

#[test]
fn a_new_champion_is_searched_first_a_step_down_on_a_seed_that_reproduces() {
    let step = sim(next(&CHAMPION, &[]));
    assert_eq!(step.champion, CHAMPION.digest());
    assert_eq!(step.challenger, at(0.4));
    assert_eq!(step.stage, Stage::First);
    // The seed is named by the champion and the challenger, so a run again is the same sim.
    assert_eq!(sim(next(&CHAMPION, &[])).seed, step.seed);
    assert_ne!(sim(next(&at(0.6), &[])).seed, step.seed);
}

/// What a sim found: `mbb` mbb/100 with a 95% half-width of 100.
fn found(mbb: f64) -> Verdict {
    Verdict {
        deals: 2_000,
        mbb_per_100: mbb,
        ci95: 100.0,
    }
}

const ABOVE: f64 = 300.0;
const LEVEL: f64 = 50.0;

/// The log after running `step` and finding `mbb`.
fn ran(step: Step, mbb: f64) -> Entry {
    Entry::Simmed {
        step,
        verdict: found(mbb),
    }
}

#[test]
fn a_step_not_above_the_champion_is_left_and_the_next_one_is_simmed() {
    let down = sim(next(&CHAMPION, &[]));
    let log = [ran(down, LEVEL)];
    let up = sim(next(&CHAMPION, &log));
    assert_eq!(up.challenger, at(0.625));
    assert_eq!(up.stage, Stage::First);
    // Both steps of the wide ring spent: the finer ring, a step down first.
    let log = [ran(down, LEVEL), ran(up, -ABOVE)];
    let finer = sim(next(&CHAMPION, &log));
    assert_eq!(finer.challenger, at(0.5 * 0.9));
}

#[test]
fn a_challenger_is_proposed_only_once_a_confirmation_on_fresh_deals_agrees() {
    let first = sim(next(&CHAMPION, &[]));
    let confirmation = sim(next(&CHAMPION, &[ran(first, ABOVE)]));
    assert_eq!(confirmation.challenger, first.challenger);
    assert_eq!(confirmation.stage, Stage::Confirmation);
    assert_ne!(
        confirmation.seed, first.seed,
        "a confirmation plays fresh deals"
    );

    let log = [ran(first, ABOVE), ran(confirmation, ABOVE)];
    assert_eq!(
        next(&CHAMPION, &log),
        Next::Propose {
            challenger: first.challenger,
            first: found(ABOVE),
            confirmation: found(ABOVE),
        }
    );
    // A confirmation that does not agree: nothing is proposed, and the search moves on.
    let log = [ran(first, ABOVE), ran(confirmation, LEVEL)];
    assert_eq!(sim(next(&CHAMPION, &log)).challenger, at(0.625));
}

#[test]
fn a_proposed_challenger_is_not_proposed_again_and_a_spent_search_says_so() {
    let mut log = Vec::new();
    loop {
        match next(&CHAMPION, &log) {
            Next::Sim(step) => log.push(ran(step, LEVEL)),
            Next::Propose { .. } => panic!("nothing was above"),
            Next::Exhausted => break,
        }
        assert!(log.len() < 100, "the search must end");
    }
    // Every step of every ring, once each.
    assert_eq!(log.len(), 6);

    let first = sim(next(&CHAMPION, &[]));
    let confirmation = sim(next(&CHAMPION, &[ran(first, ABOVE)]));
    let proposed = Entry::Proposed {
        champion: CHAMPION.digest(),
        challenger: first.challenger,
    };
    let log = [ran(first, ABOVE), ran(confirmation, ABOVE), proposed];
    assert_eq!(sim(next(&CHAMPION, &log)).challenger, at(0.625));
}

#[test]
fn a_new_champion_is_searched_afresh_and_a_step_out_of_range_is_skipped() {
    let old = sim(next(&CHAMPION, &[]));
    let promoted = at(0.4);
    let step = sim(next(&promoted, &[ran(old, LEVEL)]));
    assert_eq!(step.champion, promoted.digest());
    assert_eq!(step.challenger, at(0.4 * 0.8));
    // An exploit budget above 1 is not one the policy is defined on.
    let near_top = at(0.9);
    let down = sim(next(&near_top, &[]));
    let log = [ran(down, LEVEL)];
    assert_eq!(sim(next(&near_top, &log)).challenger, at(0.9 * 0.9));
}

#[test]
fn the_fleet_search_never_moves_a_number_the_reference_style_pins() {
    // The fleet champion plays the reference policy, which reads none of the playbook's cuts
    // and no spread: the search must not spend sims on them.
    let step = sim(next(&CHAMPION, &[]));
    let moved: Vec<&str> = step
        .challenger
        .named()
        .into_iter()
        .zip(CHAMPION.named())
        .filter(|((_, challenger), (_, champion))| challenger.to_bits() != champion.to_bits())
        .map(|((name, _), _)| name)
        .collect();
    assert_eq!(moved, ["exploit"]);
}
