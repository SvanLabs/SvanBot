//! The fleet's files: the champion every bot plays by, the proposal the canary takes up, and the
//! Arena's recorded runs.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::probe::Kind;
use svanbot_runtime::fleet::{ARENA_HISTORY, ArenaRow, ArenaRun, Fleet, Proposal};
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-fleet-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Numbers with no short decimal form: they must come back to the bit.
const CHALLENGER: Params = Params {
    exploit: 0.1 + 0.2,
    value_margin: 1.0 / 3.0,
    raise_share: 0.75,
};

fn proposal() -> Proposal {
    Proposal {
        challenger: CHALLENGER,
        kind: Kind::Guard { hands: 3_000 },
        harm_margin: 20.0,
        sim_estimate: Some(1.0 / 7.0),
        max_orbits: 3_334,
    }
}

#[test]
fn the_champion_is_the_default_until_one_is_promoted_and_comes_back_to_the_bit() {
    let fleet = Fleet::in_dir(scratch("champion"));
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
    fleet.promote(&CHALLENGER).unwrap();
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
    assert_eq!(fleet.champion().unwrap().digest(), CHALLENGER.digest());
}

#[test]
fn a_champion_file_whose_numbers_do_not_hash_to_its_digest_is_refused() {
    let dir = scratch("tampered");
    let fleet = Fleet::in_dir(&dir);
    fleet.promote(&CHALLENGER).unwrap();
    let path = dir.join("champion.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("0.75", "0.76")).unwrap();
    assert!(fleet.champion().is_err());
}

#[test]
fn a_proposal_waits_until_it_is_taken_and_is_taken_once() {
    let fleet = Fleet::in_dir(scratch("proposal"));
    assert_eq!(fleet.take_proposal(), Ok(None));
    fleet.propose(&proposal()).unwrap();
    assert_eq!(fleet.take_proposal(), Ok(Some(proposal())));
    assert_eq!(fleet.take_proposal(), Ok(None));
}

#[test]
fn a_proposal_reads_back_from_its_own_text_and_names_its_hypothesis() {
    let text = proposal().to_json();
    assert_eq!(Proposal::from_json(&text), Ok(proposal()));
    let probe = Proposal {
        kind: Kind::Probe,
        sim_estimate: None,
        ..proposal()
    };
    assert_eq!(Proposal::from_json(&probe.to_json()), Ok(probe));
    let hypothesis = proposal().hypothesis(&Params::DEFAULT);
    assert_eq!(hypothesis.champion, Params::DEFAULT.digest());
    assert_eq!(hypothesis.challenger, CHALLENGER.digest());
    assert_eq!(hypothesis.kind, Kind::Guard { hands: 3_000 });
    assert!(Proposal::from_json("{}").is_err());
}

fn run(commit: &str, seed: u64, mbb: f64) -> ArenaRun {
    ArenaRun {
        commit: commit.into(),
        seed,
        rows: vec![ArenaRow {
            opponent: "nit".into(),
            deals: 400,
            mbb_per_100: mbb,
            ci95: 250.0,
        }],
    }
}

#[test]
fn the_arena_keeps_the_latest_run_and_a_short_history_of_the_ones_before() {
    let fleet = Fleet::in_dir(scratch("arena"));
    assert_eq!(fleet.arena_runs(), Ok(Vec::new()));
    for number in 0..(ARENA_HISTORY as u64 + 3) {
        fleet
            .record_arena(&run(&format!("c{number}"), number, 1.0))
            .unwrap();
    }
    let recorded = fleet.arena_runs().unwrap();
    assert_eq!(recorded.len(), ARENA_HISTORY);
    // Newest first, so the dashboard reads the latest without searching, and the oldest fall off.
    assert_eq!(recorded[0].commit, format!("c{}", ARENA_HISTORY as u64 + 2));
    assert_eq!(recorded[ARENA_HISTORY - 1].commit, "c3");
}

#[test]
fn a_run_reads_back_from_its_own_file_unchanged_enough_to_report() {
    let dir = scratch("arena-roundtrip");
    let fleet = Fleet::in_dir(&dir);
    let run = run("abc1234", 7, 12_345.6 + 0.1 + 0.2);
    fleet.record_arena(&run).unwrap();
    // The last bit of a float is the writer's, not the run's: the run is played again from its
    // commit and seed, so a scoreboard line has to read back as what it printed, not bit for bit.
    let [read] = fleet.arena_runs().unwrap().try_into().unwrap();
    assert_eq!((read.commit, read.seed), (run.commit.clone(), run.seed));
    assert_eq!(read.rows.len(), 1);
    assert_eq!(read.rows[0].opponent, "nit");
    assert_eq!(read.rows[0].deals, 400);
    assert!((read.rows[0].mbb_per_100 - run.rows[0].mbb_per_100).abs() < 1e-6);
    assert_eq!(read.rows[0].ci95, 250.0);
    // The same run recorded twice is the same run: re-running a commit that did not change the
    // policy replaces its line rather than growing the history.
    fleet.record_arena(&run).unwrap();
    assert_eq!(fleet.arena_runs().unwrap().len(), 1);
}

#[test]
fn a_run_file_that_does_not_parse_is_refused_rather_than_misread() {
    let dir = scratch("arena-broken");
    let fleet = Fleet::in_dir(&dir);
    fleet.record_arena(&run("bad", 1, 1.0)).unwrap();
    std::fs::write(dir.join("arena.json"), "{\"runs\":[{\"commit\":\"x\"}]}").unwrap();
    assert!(fleet.arena_runs().is_err());
}
