//! The fleet's files: the champion every bot plays by, and the proposal the canary takes up.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::probe::Kind;
use svanbot_runtime::fleet::{Fleet, Proposal};
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
