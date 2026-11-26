//! A bot's ledger kept in its store, and the canary's probe from proposal to promotion.
// A test store and fleet are files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::experiment::{Budget, Claim, Experiment, Metric, StopRule};
use svanbot_learn::paired::Verdict as SimVerdict;
use svanbot_learn::probe::Kind;
use svanbot_runtime::fleet::{ExperimentStatus, Fleet, Proposal};
use svanbot_runtime::ledger::ORBIT;
use svanbot_runtime::probing::Probing;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::Store;
use svanbot_strategy::params::Params;

const CHALLENGER: Params = Params {
    exploit: 0.9,
    spread: 0.0,
    ..Params::DEFAULT
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-probing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hand(number: u32, stack: i64) -> Hand {
    Hand {
        hand_id: format!("h-{number}"),
        table_id: "t1".to_owned(),
        frames: vec![
            format!(
                r#"{{"type":"hand_start","hand_id":"h-{number}","seat":2,"dealer_seat":0,"blinds":{{"small_blind":10,"big_blind":20}}}}"#
            ),
            format!(
                r#"{{"type":"hand_result","winners":[],"final_stacks":{{"0":1960,"2":{stack}}}}}"#
            ),
        ],
        traced: Vec::new(),
    }
}

fn guard(hands: u64) -> Proposal {
    Proposal {
        challenger: CHALLENGER,
        kind: Kind::Guard { hands },
        harm_margin: 5.0,
        sim_estimate: Some(2.0),
        max_orbits: 10_000,
        against: None,
    }
}

#[test]
fn the_canary_mirrors_its_probe_while_it_runs_and_clears_it_when_it_ends() {
    let dir = scratch("mirror");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet.propose(&guard(120)).unwrap();
    let next = play(&mut canary, 0, 1, 0);
    assert_eq!(fleet.probe(), Ok(Some(guard(120))));
    assert_eq!(fleet.propose_if_free(&guard(60)), Ok(false));
    play(&mut canary, next, 1 + 20 * ORBIT, 0);
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
    assert_eq!(fleet.probe(), Ok(None));
}

#[test]
fn a_probes_verdict_is_written_into_the_experiment_ledger() {
    let dir = scratch("experiment-ledger");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    let experiment = Experiment {
        claim: Claim::ChallengerKeepsMargin {
            champion: Params::DEFAULT.digest(),
            challenger: CHALLENGER,
            sim: SimVerdict {
                deals: 100,
                mbb_per_100: -300.0,
                ci95: 100.0,
            },
        },
        metric: Metric::LiveEdge,
        budget: Budget { hands: 120 },
        stop: StopRule::Guard { margin_bb: 5.0 },
    };
    assert_eq!(fleet.open_experiment(&experiment, 400), Ok(true));
    fleet.propose(&guard(120)).unwrap();
    // The first hand takes the proposal up: the canary claims the experiment.
    let next = play(&mut canary, 0, 1, 0);
    let record = fleet.experiment_for("SurSvan").unwrap().unwrap();
    assert_eq!(record.status, ExperimentStatus::Testing);
    // The guard clears, and its verdict and evidence land in the same record.
    play(&mut canary, next, 1 + 20 * ORBIT, 0);
    let record = fleet.experiment_for("SurSvan").unwrap().unwrap();
    assert_eq!(record.status, ExperimentStatus::Cleared);
    let evidence = record.evidence.expect("the guard's reading");
    assert!(evidence.hands > 0, "the evidence counts the guard's hands");
    assert!(record.probe.is_some(), "the probe id is in the record");
}

#[test]
fn a_canary_that_opens_with_no_probe_in_its_store_clears_a_mirror_left_behind() {
    // A canary that died between mirroring a probe and storing its start would leave the learner
    // waiting on a probe nobody runs.
    let dir = scratch("stale-mirror");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.mirror_probe(&guard(120)).unwrap();
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let _canary = Probing::open(&store, fleet.clone(), true, 500);
    assert_eq!(fleet.probe(), Ok(None));
    // Only the canary owns the mirror: another bot leaves it be.
    fleet.mirror_probe(&guard(120)).unwrap();
    let other = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let _other = Probing::open(&other, fleet.clone(), false, 500);
    assert_eq!(fleet.probe(), Ok(Some(guard(120))));
}

#[test]
fn a_frozen_reference_ends_a_running_probe_without_promoting() {
    let dir = scratch("frozen-probe");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet.propose(&guard(120)).unwrap();
    let next = play(&mut canary, 0, 1, 0);
    assert!(store.open_probe().unwrap().is_some());
    fleet.freeze(600).unwrap();
    // Enough hands for the guard to clear, which after the freeze must not promote.
    let notes = (next..next + 1 + 20 * ORBIT)
        .flat_map(|number| canary.on_hand(&hand(number, 1_000_000), 2_000).notes)
        .collect::<Vec<_>>();
    assert!(
        notes.iter().any(|note| note.contains("frozen")),
        "{notes:?}"
    );
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
    assert_eq!(canary.params(), Params::DEFAULT);
    assert_eq!(fleet.probe(), Ok(None));
}

#[test]
fn a_restarted_canary_does_not_take_a_probe_up_after_the_freeze() {
    let dir = scratch("frozen-restart");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet.propose(&guard(120)).unwrap();
    let next = play(&mut canary, 0, 1 + 5 * ORBIT, 0);
    drop(canary);
    fleet.freeze(600).unwrap();

    let (mut restarted, notes) = Probing::open(&store, fleet.clone(), true, 500);
    assert!(
        notes.iter().any(|note| note.contains("frozen")),
        "{notes:?}"
    );
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.probe(), Ok(None));
    play(&mut restarted, next, 1 + 20 * ORBIT, 0);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
    assert_eq!(restarted.params(), Params::DEFAULT);
}

#[test]
fn a_proposal_measured_against_another_champion_is_dropped_unplayed() {
    let dir = scratch("stale");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let promoted = Params {
        exploit: 0.4,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.promote(&promoted).unwrap();
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet
        .propose(&Proposal {
            against: Some(Params::DEFAULT.digest()),
            ..guard(120)
        })
        .unwrap();
    let notes = canary.on_hand(&hand(0, 1_000_000), 1_000).notes;
    assert!(
        notes.iter().any(|note| note.contains("another champion")),
        "{notes:?}"
    );
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.probe(), Ok(None));
    assert_eq!(canary.params(), promoted);
    // Measured against today's champion, it is played.
    fleet
        .propose(&Proposal {
            against: Some(promoted.digest()),
            ..guard(120)
        })
        .unwrap();
    canary.on_hand(&hand(1, 1_000_000), 1_000);
    assert!(store.open_probe().unwrap().is_some());
}

/// Plays `hands` hands numbered from `from`, in which a hand played by the challenger wins
/// `challenger` chips and any other breaks even. Returns the next hand's number.
fn play(probing: &mut Probing<'_>, from: u32, hands: u32, challenger: i64) -> u32 {
    let mut stack = 1_000_000 + i64::from(from);
    for number in from..from + hands {
        if probing.params() == CHALLENGER {
            stack += challenger + i64::from(number % 3) * 20 - 20;
        }
        probing.on_hand(&hand(number, stack), 1_000);
    }
    from + hands
}

#[test]
fn every_bot_keeps_what_its_hands_won() {
    let dir = scratch("keeps");
    let store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut probing = Probing::open(&store, Fleet::in_dir(dir.join("fleet")), false, 500).0;
    probing.on_hand(&hand(1, 2000), 1_000);
    probing.on_hand(&hand(2, 2060), 1_000);
    let played = store.played().unwrap();
    assert_eq!(played.len(), 1);
    assert_eq!((played[0].net, played[0].orbit), (60, None));
}

#[test]
fn only_the_canary_takes_up_a_proposal() {
    let dir = scratch("only-canary");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.propose(&guard(120)).unwrap();
    let store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut probing = Probing::open(&store, fleet.clone(), false, 500).0;
    play(&mut probing, 0, 30, 0);
    assert_eq!(store.open_probe().unwrap(), None);
    assert!(fleet.take_proposal().unwrap().is_some());
}

#[test]
fn a_guard_that_clears_promotes_the_challenger_and_the_fleet_adopts_it() {
    let dir = scratch("clears");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    let other_store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut other = Probing::open(&other_store, fleet.clone(), false, 500).0;

    fleet.propose(&guard(120)).unwrap();
    // The first hand is not counted, and the canary takes the proposal up after it.
    let next = play(&mut canary, 0, 1, 0);
    assert!(store.open_probe().unwrap().is_some());
    // Mid-guard, the rest of the fleet still plays the champion.
    let next = play(&mut canary, next, 60, 0);
    play(&mut other, 0, 2, 0);
    assert_eq!(other.params(), Params::DEFAULT);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));

    let notes = (next..next + 80)
        .flat_map(|number| canary.on_hand(&hand(number, 1_000_000), 2_000).notes)
        .collect::<Vec<_>>();
    assert!(
        notes.iter().any(|note| note.contains("cleared")),
        "{notes:?}"
    );
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
    assert_eq!(canary.champion(), CHALLENGER);
    // The others adopt between hands. A reference-arm hand still plays the reference.
    play(&mut other, 2, 1, 0);
    assert_eq!(other.champion(), CHALLENGER);
}

#[test]
fn a_guard_that_finds_harm_rolls_back_and_the_fleet_never_sees_the_challenger() {
    let dir = scratch("harm");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet.propose(&guard(30_000)).unwrap();
    // The challenger loses two big blinds a hand.
    play(&mut canary, 0, 1 + 400 * ORBIT, -40);
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
    assert_eq!(canary.params(), Params::DEFAULT);
}

#[test]
fn a_restarted_canary_takes_its_probe_up_where_the_store_left_it() {
    let dir = scratch("restart");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    fleet.propose(&guard(120)).unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    // 15 full orbits of the guard's 20.
    let next = play(&mut canary, 0, 1 + 15 * ORBIT, 0);
    drop(canary);
    // A mirror lost while the canary was down is written again when it takes the probe up.
    fleet.end_probe().unwrap();

    let (mut canary, notes) = Probing::open(&store, fleet.clone(), true, 500);
    assert_eq!(fleet.probe(), Ok(Some(guard(120))));
    assert!(
        notes.iter().any(|note| note.contains("15 orbits")),
        "{notes:?}"
    );
    // One uncounted hand at the table again, then the five orbits left.
    play(&mut canary, next, 1 + 5 * ORBIT, 0);
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
}

#[test]
fn a_promotion_taken_back_by_hand_reaches_the_canary_too() {
    let dir = scratch("taken-back");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.promote(&CHALLENGER).unwrap();
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    assert_eq!(canary.champion(), CHALLENGER);
    std::fs::remove_file(dir.join("fleet").join("champion.json")).unwrap();
    play(&mut canary, 0, 1, 0);
    assert_eq!(canary.champion(), Params::DEFAULT);
}

#[test]
fn a_bot_plays_its_own_slot_and_a_slot_update_waits_for_the_next_hand() {
    let dir = scratch("own-slot");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let slot_a = Params {
        exploit: 0.2,
        spread: 0.0,
        ..Params::DEFAULT
    };
    let slot_b = Params {
        exploit: 0.7,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.set_champion("A", &slot_a).unwrap();
    fleet.set_champion("B", &slot_b).unwrap();
    let a_store = Store::open(&dir.join("A.db"), "A").unwrap();
    let b_store = Store::open(&dir.join("B.db"), "B").unwrap();
    let mut a = Probing::open(&a_store, fleet.clone(), false, 500).0;
    let b = Probing::open(&b_store, fleet.clone(), false, 500).0;
    // A's first standing orbit is its policy's, so it plays its slot. B's is the reference,
    // so B's hand in play is the reference until that orbit ends; its slot is still its own.
    assert_eq!(a.params(), slot_a);
    assert_eq!(b.champion(), slot_b);
    assert!(b.on_reference());
    assert_eq!(b.params(), Params::DEFAULT);
    let updated = Params {
        exploit: 0.3,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.set_champion("A", &updated).unwrap();
    // Between hands only: the hand in play keeps the champion it began with.
    assert_eq!(a.params(), slot_a);
    assert_eq!(b.champion(), slot_b);
    play(&mut a, 0, 1, 0);
    assert_eq!(a.params(), updated);
    assert_eq!(b.champion(), slot_b);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
}

#[test]
fn a_bot_without_a_slot_plays_and_adopts_the_fleet_champion() {
    let dir = scratch("no-slot");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut probing = Probing::open(&store, fleet.clone(), false, 500).0;
    assert_eq!(probing.params(), Params::DEFAULT);
    fleet.promote(&CHALLENGER).unwrap();
    assert_eq!(probing.params(), Params::DEFAULT);
    play(&mut probing, 0, 1, 0);
    // The first hand is uncounted and MegaSvan's first orbit is the standing reference, so the
    // hand in play stays the reference while the champion it owns has moved.
    assert!(probing.on_reference());
    assert_eq!(probing.params(), Params::DEFAULT);
    assert_eq!(probing.champion(), CHALLENGER);
}

#[test]
fn a_slot_that_cannot_be_read_leaves_the_bot_on_the_fleet_champion() {
    let dir = scratch("bad-slot");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.promote(&CHALLENGER).unwrap();
    fleet
        .set_champion(
            "A",
            &Params {
                exploit: 0.3,
                spread: 0.0,
                ..Params::DEFAULT
            },
        )
        .unwrap();
    let path = dir.join("fleet").join("champions.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"0.3\"", "\"0.4\"")).unwrap();
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    let (probing, notes) = Probing::open(&store, fleet.clone(), false, 500);
    assert_eq!(probing.params(), CHALLENGER);
    assert!(
        notes.iter().any(|note| note.contains("champion")),
        "{notes:?}"
    );
}

fn ended_calibration(
    name: &str,
    sim_estimate: Option<f64>,
) -> Vec<svanbot_learn::calibration::Point> {
    let dir = scratch(name);
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet
        .propose(&Proposal {
            sim_estimate,
            ..guard(120)
        })
        .unwrap();
    play(&mut canary, 0, 2 + 20 * ORBIT, 0);
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
    assert!(fleet.probe().unwrap().is_none());
    fleet.calibration().unwrap()
}

#[test]
fn a_probe_that_ends_with_a_sim_estimate_leaves_a_calibration_point() {
    let points = ended_calibration("calibrated", Some(2.));
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].sim, 2.);
    assert!(points[0].live.is_finite() && points[0].radius.is_finite());
}

#[test]
fn a_probe_with_no_sim_estimate_leaves_none() {
    assert!(ended_calibration("no-calibration", None).is_empty());
}
