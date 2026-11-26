//! A champion slot for one bot: its own parameter set, seeded from the reference and falling
//! back to the fleet champion while it has none.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::{LOOSE, TIGHT};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-champions-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn tight() -> Assignment {
    Assignment {
        kind: PolicyKind::Solving,
        style: &TIGHT,
    }
}

fn loose() -> Assignment {
    Assignment {
        kind: PolicyKind::Solving,
        style: &LOOSE,
    }
}

/// A parameter set that is not the default.
const CHALLENGER: Params = Params {
    exploit: 0.1 + 0.2,
    spread: 0.0,
    ..Params::DEFAULT
};

#[test]
fn a_new_slot_is_the_reference_and_each_bot_reads_only_its_own() {
    let dir = scratch("seed");
    let fleet = Fleet::in_dir(&dir);
    fleet.assign("A", tight(), 1).unwrap();
    fleet.assign("B", loose(), 1).unwrap();
    let reference = Params {
        exploit: 0.4,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.promote(&reference).unwrap();
    // Before any slot, both bots read the fleet champion, and no file was made.
    assert_eq!(fleet.champion_slot("A"), Ok(None));
    assert_eq!(fleet.champion_for("A"), Ok(reference));
    assert!(!dir.join("champions.json").exists());
    // Seeded under two different styles, a slot starts at the reference's numbers brought inside
    // its style: Grinder's bounds include them as they are; Gambler's start at its own end of the
    // learned shares (a bluff share, a lower dry c-bet, a higher wet one).
    assert_eq!(fleet.seed_champion("A"), Ok(reference));
    assert_eq!(
        fleet.seed_champion("B"),
        Ok(Params {
            three_bet_bluff: 0.04,
            cbet_dry: 0.5,
            cbet_wet: 0.4,
            ..reference
        })
    );
    // Distinct slots stay distinct, and the fleet champion does not move.
    let slot_a = Params {
        exploit: 0.2,
        spread: 0.0,
        ..Params::DEFAULT
    };
    let slot_b = Params {
        exploit: 0.9,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.set_champion("A", &slot_a).unwrap();
    fleet.set_champion("B", &slot_b).unwrap();
    assert_eq!(fleet.champion_slot("A"), Ok(Some(slot_a)));
    assert_eq!(fleet.champion_slot("B"), Ok(Some(slot_b)));
    assert_eq!(fleet.champion_for("A"), Ok(slot_a));
    assert_eq!(fleet.champion_for("B"), Ok(slot_b));
    // A bot with no slot still reads the fleet champion.
    assert_eq!(fleet.champion_slot("C"), Ok(None));
    assert_eq!(fleet.champion_for("C"), Ok(reference));
    assert_eq!(fleet.champion(), Ok(reference));
    // A slot is keyed by the exact bot name, as an assignment is.
    assert_eq!(fleet.champion_slot("a"), Ok(None));
}

#[test]
fn the_slots_are_listed_by_bot_name_for_the_learner_to_rotate_through() {
    let fleet = Fleet::in_dir(scratch("list"));
    assert_eq!(fleet.champion_slots(), Ok(Vec::new()));
    let a = Params {
        exploit: 0.2,
        spread: 0.0,
        ..Params::DEFAULT
    };
    let b = Params {
        exploit: 0.7,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.set_champion("B", &b).unwrap();
    fleet.set_champion("A", &a).unwrap();
    assert_eq!(
        fleet.champion_slots(),
        Ok(vec![("A".into(), a), ("B".into(), b)])
    );
}

#[test]
fn a_fleet_with_no_champions_file_keeps_playing_the_fleet_champion() {
    // The files a fleet has before Phase 2 are enough: champion.json for the fleet, plays.json
    // for the assignments, and no champions.json at all.
    let dir = scratch("legacy");
    let fleet = Fleet::in_dir(&dir);
    fleet.assign("A", tight(), 1).unwrap();
    fleet.promote(&CHALLENGER).unwrap();
    assert!(!dir.join("champions.json").exists());
    assert_eq!(fleet.champion_slot("A"), Ok(None));
    assert_eq!(fleet.champion_for("A"), Ok(CHALLENGER));
}

#[test]
fn a_slot_is_seeded_once_and_its_numbers_must_hash_to_their_digest() {
    let dir = scratch("tamper");
    let fleet = Fleet::in_dir(&dir);
    let reference = Params {
        exploit: 0.4,
        spread: 0.0,
        ..Params::DEFAULT
    };
    fleet.promote(&reference).unwrap();
    fleet.seed_champion("A").unwrap();
    fleet
        .set_champion(
            "A",
            &Params {
                exploit: 0.2,
                spread: 0.0,
                ..Params::DEFAULT
            },
        )
        .unwrap();
    // Seeding fills a new slot; it never replaces the champion a bot has learned.
    assert!(fleet.seed_champion("A").unwrap_err().contains("already"));
    assert_eq!(
        fleet.champion_slot("A"),
        Ok(Some(Params {
            exploit: 0.2,
            spread: 0.0,
            ..Params::DEFAULT
        }))
    );
    // A hand-written number that does not hash to its digest is refused, as champion.json is.
    let path = dir.join("champions.json");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("\"0.2\"", "\"0.3\"")).unwrap();
    assert!(fleet.champion_slot("A").is_err());
    // The fleet champion is a separate file and is unaffected.
    assert_eq!(fleet.champion(), Ok(reference));
}
