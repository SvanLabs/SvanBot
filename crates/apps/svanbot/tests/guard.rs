//! The live guard and the policy-assignment fixtures it needs.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};

use svanbot::play::guard::{Guarded, guard, judge};
use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_runtime::ledger::Played;
use svanbot_runtime::store::Store;
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::TIGHT;

fn fixture(name: &str, _store: bool) -> (PathBuf, Fleet) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let fleet = Fleet::in_dir(dir.join("fleet"));
    (dir, fleet)
}

fn hands(dir: &Path, name: &str, count: i64, net: i64) {
    let store = Store::open(&dir.join(format!("{name}.db")), name).unwrap();
    for at in 1..=count {
        store
            .append_played(
                &Played {
                    hand_id: format!("hand-{at}"),
                    table_id: "scratch".into(),
                    digest: Params::DEFAULT.digest(),
                    net,
                    luck: 0.,
                    big_blind: 20,
                    orbit: None,
                    reference: None,
                },
                at,
            )
            .unwrap();
    }
}

fn assign(fleet: &Fleet, bot: &str) {
    fleet
        .assign(
            bot,
            Assignment {
                kind: PolicyKind::Solving,
                style: &TIGHT,
            },
            1,
        )
        .unwrap();
}

#[test]
fn the_guard_waits_for_the_hands_before_it_says_anything() {
    assert!(matches!(
        judge(&[-10.; 10], &[0.; 10], 3_000, 20.),
        Guarded::NotYet(_)
    ));
}

#[test]
fn a_styled_bot_clearly_losing_to_the_control_trips() {
    assert!(matches!(
        judge(&[-0.4; 3_000], &[0.; 3_000], 3_000, 20.),
        Guarded::Trips(_)
    ));
}

#[test]
fn a_styled_bot_level_with_the_control_holds() {
    assert!(matches!(
        judge(&[0.; 20], &[0.; 20], 20, 20.),
        Guarded::Holds(_)
    ));
}

#[test]
fn an_empty_control_is_not_yet_never_a_trip() {
    assert!(matches!(
        judge(&[-0.4; 20], &[], 20, 20.),
        Guarded::NotYet(_)
    ));
}

#[test]
fn the_guard_never_counts_the_styled_bot_in_its_own_control() {
    let (dir, fleet) = fixture("guard-control", false);
    hands(&dir, "A", 20, -8);
    hands(&dir, "B", 20, 0);
    hands(&dir, "Control", 19, 0);
    assign(&fleet, "A");
    assign(&fleet, "B");
    let held = guard(&fleet, &dir, 20, 20.).unwrap();
    assert_eq!(held.len(), 2);
    assert!(
        held.iter()
            .all(|(_, result)| matches!(result, Guarded::NotYet(_)))
    );
    assert_eq!(fleet.assignments().unwrap().len(), 2);
}

#[test]
fn a_trip_reverts_the_assignment_and_a_hold_leaves_it() {
    let (dir, fleet) = fixture("guard-revert", false);
    hands(&dir, "A", 20, -8);
    hands(&dir, "B", 20, 0);
    hands(&dir, "Control", 20, 0);
    assign(&fleet, "A");
    assign(&fleet, "B");
    let held = guard(&fleet, &dir, 20, 20.).unwrap();
    assert!(matches!(held[0].1, Guarded::Trips(_)));
    assert!(matches!(held[1].1, Guarded::Holds(_)));
    assert!(fleet.assignment("A").unwrap().is_none());
    assert!(fleet.assignment("B").unwrap().is_some());
}
