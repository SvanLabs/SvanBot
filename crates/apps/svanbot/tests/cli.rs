//! `svanbot play`'s command-line surface: the fleet files an operator moves by hand.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use svanbot::play::assign;
use svanbot_runtime::fleet::{Check, Fleet};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::TIGHT;

fn fixture(name: &str) -> (PathBuf, Fleet) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Svanar.db"), "").unwrap();
    let fleet = Fleet::in_dir(dir.join("fleet"));
    (dir, fleet)
}

fn in_fleet(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .env("SVANBOT_FLEET", dir.join("fleet"))
        .arg("play")
        .args(args)
        .output()
        .unwrap()
}

fn check() -> Check {
    Check {
        kind: PolicyKind::Solving,
        style: "tight".into(),
        reference: Params::DEFAULT.digest(),
        seed: 7,
        iterations: Some(20),
        bouts: 60,
        admitted: true,
        heads_up: "heads-up evidence".into(),
        six_max: "6-max evidence".into(),
        checked_ms: 1,
    }
}

#[test]
fn freeze_records_the_reference_and_rebase_moves_it_on_purpose() {
    let (dir, fleet) = fixture("cli-freeze");
    fleet.promote(&Params { exploit: 0.3 }).unwrap();
    let output = in_fleet(&dir, &["freeze"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frozen = fleet.reference().unwrap().expect("the reference is frozen");
    assert_eq!(frozen.params, Params { exploit: 0.3 });
    assert!(String::from_utf8_lossy(&output.stdout).contains("frozen"));

    let rebase = in_fleet(&dir, &["rebase", "exploit=0.6"]);
    assert!(
        rebase.status.success(),
        "{}",
        String::from_utf8_lossy(&rebase.stderr)
    );
    assert_eq!(
        fleet.reference().unwrap().unwrap().params,
        Params { exploit: 0.6 }
    );
    assert_eq!(fleet.rebases().unwrap(), vec![frozen]);
    // Status names the freeze and its cutover.
    let status = in_fleet(&dir, &["status"]);
    assert!(status.status.success());
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("frozen at"),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    // A rebase names the numbers it moves: none is a usage error, not a silent re-freeze.
    let none = in_fleet(&dir, &["rebase"]);
    assert_eq!(none.status.code(), Some(2));
}

#[test]
fn assign_without_a_check_is_refused_with_exit_2() {
    let (dir, _) = fixture("cli-refused");
    let output = in_fleet(&dir, &["assign", "Svanar", "solving", "tight"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("check"));
}

#[test]
fn status_lists_assignments_and_checks_and_marks_a_stale_check() {
    let (dir, fleet) = fixture("cli-status");
    fleet.record_check(&check()).unwrap();
    assign(&fleet, &dir, "Svanar", PolicyKind::Solving, &TIGHT, 2).unwrap();
    fleet.promote(&Params { exploit: 0.3 }).unwrap();
    let output = in_fleet(&dir, &["status"]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "Svanar",
        "solving/tight",
        "stale",
        "heads-up evidence",
        "6-max evidence",
    ] {
        assert!(text.contains(expected), "{text}");
    }
}
