#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot::live_policy::LivePolicy;
use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_runtime::probing::Probing;
use svanbot_runtime::store::Store;
use svanbot_strategy::{kind::PolicyKind, params::Params, style::TIGHT};

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("svanbot-live-policy-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const TIGHT_PLAY: Assignment = Assignment {
    kind: PolicyKind::Solving,
    style: &TIGHT,
};

#[test]
fn with_no_assignment_a_bot_plays_the_reference_policy() {
    let (live, notes) = LivePolicy::open(Fleet::in_dir(scratch("reference")), "A");
    assert!(notes.is_empty());
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/standard");
}

#[test]
fn an_assignment_is_taken_up_between_hands_and_a_revert_is_too() {
    let fleet = Fleet::in_dir(scratch("refresh"));
    let (live, _) = LivePolicy::open(fleet.clone(), "A");
    fleet.assign("A", TIGHT_PLAY, 1).unwrap();
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/standard");
    assert!(live.refresh().unwrap().contains("solving/tight"));
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/tight");
    fleet.revert("A").unwrap();
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/tight");
    assert!(live.refresh().unwrap().contains("reference"));
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/standard");
}

#[test]
fn a_refused_file_plays_the_reference_and_says_so_once() {
    let dir = scratch("refused");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plays.json");
    std::fs::write(&path, "garbage").unwrap();
    let (live, notes) = LivePolicy::open(Fleet::in_dir(dir), "A");
    assert!(notes[0].contains("refused"));
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/standard");
    std::fs::write(&path, "").unwrap();
    assert!(live.refresh().is_none());
    std::fs::write(&path, r#"{"bots":{}}"#).unwrap();
    assert!(live.refresh().unwrap().contains("reference"));
    assert!(live.refresh().is_none());
}

#[test]
fn another_bots_entry_is_not_mine() {
    let fleet = Fleet::in_dir(scratch("another-bot"));
    let (live, _) = LivePolicy::open(fleet.clone(), "A");
    fleet.assign("B", TIGHT_PLAY, 1).unwrap();
    assert!(live.refresh().is_none());
    assert_eq!(live.policy(Params::DEFAULT).name(), "solving/standard");
}

#[test]
fn the_champions_numbers_ride_along() {
    let fleet = Fleet::in_dir(scratch("champion"));
    fleet.assign("A", TIGHT_PLAY, 1).unwrap();
    let (live, _) = LivePolicy::open(fleet, "A");
    let params = Params {
        exploit: 0.3,
        spread: 0.0,
    };
    assert_eq!(live.policy(params).params, params);
}

#[test]
fn a_seeded_slot_plays_the_bots_style_and_its_own_numbers() {
    let dir = scratch("slot");
    let fleet = Fleet::in_dir(&dir);
    let reference = Params {
        exploit: 0.4,
        spread: 0.0,
    };
    fleet.assign("A", TIGHT_PLAY, 1).unwrap();
    fleet.promote(&reference).unwrap();
    assert_eq!(fleet.seed_champion("A"), Ok(reference));
    let slot = Params {
        exploit: 0.2,
        spread: 0.0,
    };
    fleet.set_champion("A", &slot).unwrap();
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    let (live, _) = LivePolicy::open(fleet.clone(), "A");
    let probing = Probing::open(&store, fleet, false, 0).0;
    let policy = live.policy(probing.params());
    assert_eq!(policy.name(), "solving/tight");
    // The style pins the width; the slot carries the numbers the bot learns.
    assert_eq!(policy.style.open_scale, 0.75);
    assert_eq!(policy.params, slot);
}

#[test]
fn a_bot_without_a_slot_plays_the_fleet_champion_exactly() {
    let dir = scratch("no-slot");
    let fleet = Fleet::in_dir(&dir);
    let champion = Params {
        exploit: 0.3,
        spread: 0.0,
    };
    fleet.promote(&champion).unwrap();
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    let (live, notes) = LivePolicy::open(fleet.clone(), "A");
    assert!(notes.is_empty());
    let probing = Probing::open(&store, fleet, false, 0).0;
    assert_eq!(probing.params(), champion);
    let policy = live.policy(probing.params());
    let reference = svanbot_strategy::policy::Policy::reference(champion);
    assert_eq!(policy.kind, reference.kind);
    assert_eq!(policy.style.name, reference.style.name);
    assert_eq!(policy.params, reference.params);
}
