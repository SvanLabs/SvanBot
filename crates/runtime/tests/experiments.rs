//! The experiment ledger (V11-129): one a bot, five across the fleet, a challenger never asked
//! twice, and the hypothesis, evidence and verdict kept together across a restart.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::experiment::{Budget, Claim, Experiment, Metric, Outcome, StopRule};
use svanbot_learn::paired::Verdict;
use svanbot_runtime::fleet::{Evidence, ExperimentStatus, Fleet};
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn params(exploit: f64) -> Params {
    Params {
        exploit,
        ..Params::DEFAULT
    }
}

fn experiment(challenger: Params) -> Experiment {
    Experiment {
        claim: Claim::ChallengerKeepsMargin {
            champion: Params::DEFAULT.digest(),
            challenger,
            sim: Verdict {
                deals: 100,
                mbb_per_100: -300.0,
                ci95: 100.0,
            },
        },
        metric: Metric::LiveEdge,
        budget: Budget { hands: 3_000 },
        stop: StopRule::Guard { margin_bb: 20.0 },
    }
}

fn evidence(verdict: &str, edge: f64) -> Evidence {
    Evidence {
        verdict: verdict.into(),
        edge,
        radius: 100.0,
        hands: 3_000,
    }
}

#[test]
fn a_waiting_experiment_is_claimed_by_its_bot_and_keeps_the_verdict() {
    let dir = scratch("experiments-claim");
    let fleet = Fleet::in_dir(&dir);
    let written = experiment(params(0.4));
    assert_eq!(fleet.open_experiment(&written, 1_000), Ok(true));
    // A restart is a fresh Fleet over the same directory.
    let restarted = Fleet::in_dir(&dir);
    assert_eq!(
        restarted.waiting_experiment().unwrap().unwrap().experiment,
        written
    );
    assert_eq!(
        restarted.claim_experiment("A", &params(0.4).digest(), 2_000),
        Ok(true)
    );
    assert_eq!(restarted.waiting_experiment(), Ok(None));
    let record = restarted.experiment_for("A").unwrap().unwrap();
    assert_eq!(record.status, ExperimentStatus::Testing);
    assert_eq!(record.bot.as_deref(), Some("A"));

    let note = restarted
        .record_experiment(
            "A",
            Some(7),
            Outcome::Cleared,
            evidence("cleared", 42.0),
            3_000,
        )
        .unwrap()
        .expect("a note");
    assert!(note.contains("cleared"), "the note names the verdict");
    let reloaded = Fleet::in_dir(&dir);
    let record = reloaded.experiment_for("A").unwrap().unwrap();
    assert_eq!(record.status, ExperimentStatus::Cleared);
    assert_eq!(record.probe, Some(7));
    assert_eq!(record.decided_ms, Some(3_000));
    assert_eq!(record.evidence.unwrap().edge, 42.0);
    assert!(!record.status.active());
}

#[test]
fn one_experiment_a_bot_and_five_across_the_fleet() {
    let dir = scratch("experiments-cap");
    let fleet = Fleet::in_dir(&dir);
    let first = experiment(params(0.4));
    assert_eq!(fleet.open_experiment(&first, 1), Ok(true));
    assert_eq!(
        fleet.open_experiment(&first, 2),
        Ok(false),
        "a second waiting experiment is refused"
    );
    assert_eq!(
        fleet.claim_experiment("A", &params(0.4).digest(), 3),
        Ok(true)
    );
    // A is now testing; the next waiting experiment may not be claimed by A, but B may.
    let second = experiment(params(0.3));
    assert_eq!(fleet.open_experiment(&second, 4), Ok(true));
    assert_eq!(
        fleet.claim_experiment("A", &params(0.3).digest(), 5),
        Ok(false),
        "A already has a live experiment"
    );
    assert_eq!(
        fleet.claim_experiment("B", &params(0.3).digest(), 6),
        Ok(true)
    );
    // Five live (A, B, C, D, E), then the sixth waits.
    for (at, bot) in ["C", "D", "E"].into_iter().enumerate() {
        assert_eq!(
            fleet.open_experiment(&experiment(params(0.2 + at as f64 / 100.0)), 10 + at as i64),
            Ok(true)
        );
        assert_eq!(
            fleet.claim_experiment(bot, &params(0.2 + at as f64 / 100.0).digest(), 20),
            Ok(true)
        );
    }
    assert_eq!(fleet.active_experiments(), Ok(5));
    assert_eq!(
        fleet.open_experiment(&experiment(params(0.9)), 30),
        Ok(false),
        "five live is the cap"
    );
}

#[test]
fn a_challenger_already_asked_about_is_never_asked_twice() {
    let dir = scratch("experiments-tried");
    let fleet = Fleet::in_dir(&dir);
    fleet.open_experiment(&experiment(params(0.4)), 1).unwrap();
    assert!(
        fleet
            .tried_challengers()
            .unwrap()
            .contains(&params(0.4).digest())
    );
    // Retiring a waiting experiment keeps its question in the ledger.
    assert_eq!(fleet.retire_waiting("the champion moved", 2), Ok(true));
    assert_eq!(fleet.waiting_experiment(), Ok(None));
    assert!(
        fleet
            .tried_challengers()
            .unwrap()
            .contains(&params(0.4).digest())
    );
    assert_eq!(
        fleet.open_experiment(&experiment(params(0.5)), 3),
        Ok(true),
        "the next question may be asked"
    );
}

#[test]
fn a_verdict_by_another_bots_probe_does_not_write_this_bots_ledger() {
    let dir = scratch("experiments-other");
    let fleet = Fleet::in_dir(&dir);
    fleet.open_experiment(&experiment(params(0.4)), 1).unwrap();
    fleet
        .claim_experiment("A", &params(0.4).digest(), 2)
        .unwrap();
    assert_eq!(
        fleet.record_experiment(
            "B",
            Some(1),
            Outcome::Harmful,
            evidence("harmful", -30.0),
            3
        ),
        Ok(None),
        "B has no live experiment"
    );
    assert_eq!(
        fleet.experiment_for("A").unwrap().unwrap().status,
        ExperimentStatus::Testing
    );
}
