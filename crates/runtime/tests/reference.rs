//! The frozen reference (V11-78): the fleet champion at cutover, moved only by a deliberate
//! rebase and never by a bot's own slot.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::probe::Kind;
use svanbot_runtime::fleet::{Fleet, Proposal};
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const FIRST: Params = Params {
    exploit: 0.3,
    spread: 0.0,
};
const SECOND: Params = Params {
    exploit: 0.7,
    spread: 0.0,
};

#[test]
fn a_freeze_records_the_champion_in_play_with_its_digest_and_cutover_and_a_restart_keeps_it() {
    let dir = scratch("freeze");
    let fleet = Fleet::in_dir(&dir);
    fleet.promote(&FIRST).unwrap();
    let frozen = fleet.freeze(1_000).unwrap();
    assert_eq!(frozen.params, FIRST);
    assert_eq!(frozen.cutover_ms, 1_000);
    assert_eq!(frozen.digest(), FIRST.digest());
    // A restart is a fresh Fleet over the same directory.
    let restarted = Fleet::in_dir(&dir);
    assert_eq!(restarted.reference(), Ok(Some(frozen)));
    assert_eq!(restarted.champion(), Ok(FIRST));
    // The file records the digest and the cutover time.
    let text = std::fs::read_to_string(dir.join("reference.json")).unwrap();
    assert!(
        text.contains(&FIRST.digest().to_string()),
        "the reference file does not record the digest"
    );
    assert!(
        text.contains("\"cutover_ms\":1000"),
        "the reference file does not record the cutover time"
    );
}

fn proposal() -> Proposal {
    Proposal {
        challenger: SECOND,
        kind: Kind::Guard { hands: 3_000 },
        harm_margin: 20.0,
        sim_estimate: Some(1.0),
        max_orbits: 3_334,
        against: Some(FIRST.digest()),
    }
}

#[test]
fn a_freeze_stops_fleet_wide_promotions_and_proposals_and_drops_one_waiting() {
    let fleet = Fleet::in_dir(scratch("stopped"));
    fleet.promote(&FIRST).unwrap();
    fleet.propose(&proposal()).unwrap();
    fleet.freeze(1_000).unwrap();
    // A proposal waiting at cutover is not taken up.
    assert_eq!(fleet.waiting(), Ok(None));
    assert_eq!(fleet.take_proposal(), Ok(None));
    // Neither a promotion nor a new proposal may move or restart the fleet-wide path.
    assert!(fleet.promote(&SECOND).unwrap_err().contains("frozen"));
    assert!(fleet.propose(&proposal()).unwrap_err().contains("frozen"));
    assert!(
        fleet
            .propose_if_free(&proposal())
            .unwrap_err()
            .contains("frozen")
    );
    assert_eq!(fleet.champion(), Ok(FIRST));
}

#[test]
fn a_deliberate_rebase_moves_the_reference_and_keeps_the_old_one() {
    let dir = scratch("rebase");
    let fleet = Fleet::in_dir(&dir);
    fleet.promote(&FIRST).unwrap();
    let frozen = fleet.freeze(1_000).unwrap();
    let rebased = fleet.rebase(&SECOND, 2_000).unwrap();
    assert_eq!(rebased.params, SECOND);
    assert_eq!(rebased.cutover_ms, 2_000);
    assert_eq!(fleet.champion(), Ok(SECOND));
    let history = fleet.rebases().unwrap();
    assert_eq!(history, vec![frozen]);
    // A restart keeps both the rebased reference and its history.
    let restarted = Fleet::in_dir(&dir);
    assert_eq!(restarted.reference(), Ok(Some(rebased)));
    assert_eq!(restarted.rebases(), Ok(vec![frozen]));
    // A second rebase keeps the newest first.
    let third = Params {
        exploit: 0.9,
        spread: 0.0,
    };
    fleet.rebase(&third, 3_000).unwrap();
    assert_eq!(
        fleet
            .rebases()
            .unwrap()
            .into_iter()
            .map(|reference| reference.params)
            .collect::<Vec<_>>(),
        vec![SECOND, FIRST]
    );
}

#[test]
fn a_rebase_before_the_freeze_is_refused() {
    let fleet = Fleet::in_dir(scratch("rebase-open"));
    assert!(
        fleet
            .rebase(&SECOND, 1_000)
            .unwrap_err()
            .contains("not frozen")
    );
    assert_eq!(fleet.reference(), Ok(None));
}

#[test]
fn a_second_freeze_is_refused_and_a_tampered_reference_is_refused() {
    let dir = scratch("tamper");
    let fleet = Fleet::in_dir(&dir);
    fleet.promote(&FIRST).unwrap();
    fleet.freeze(1_000).unwrap();
    assert!(fleet.freeze(2_000).unwrap_err().contains("already frozen"));
    // A hand-written number that does not hash to its digest is refused, as champion.json is.
    let path = dir.join("reference.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"0.3\"", "\"0.4\"")).unwrap();
    assert!(fleet.champion().is_err());
    assert!(fleet.reference().is_err());
}

#[test]
fn a_bot_s_slot_moves_without_moving_the_frozen_reference() {
    let fleet = Fleet::in_dir(scratch("slot"));
    fleet.promote(&FIRST).unwrap();
    fleet.freeze(1_000).unwrap();
    fleet.set_champion("A", &SECOND).unwrap();
    assert_eq!(fleet.champion_slot("A"), Ok(Some(SECOND)));
    assert_eq!(fleet.champion_for("A"), Ok(SECOND));
    assert_eq!(fleet.champion(), Ok(FIRST));
    assert_eq!(fleet.reference().unwrap().unwrap().params, FIRST);
    assert!(fleet.rebases().unwrap().is_empty());
}
