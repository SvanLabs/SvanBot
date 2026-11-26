#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::process::{Command, Output};

fn play(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .arg("play")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn play_with_no_subcommand_says_its_usage_and_exits_2() {
    let result = play(&[]);
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("svanbot play arena"));
}

#[test]
fn an_unknown_kind_or_style_is_refused_before_anything_plays() {
    for args in [
        ["arena", "unknown", "standard", "2"],
        ["arena", "solving", "unknown", "2"],
    ] {
        let result = play(&args);
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&result.stderr);
        for name in ["solving", "pricing", "standard", "tight", "loose"] {
            assert!(stderr.contains(name), "{stderr}");
        }
        assert!(!stderr.contains("record the start"), "{stderr}");
    }
}

use std::path::{Path, PathBuf};
use svanbot::play::{assign, judge};
use svanbot_learn::margin::{Admission, MARGIN};
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Check, Fleet, League};
use svanbot_strategy::{
    kind::PolicyKind,
    params::Params,
    style::{STANDARD, TIGHT},
};

fn run(values: [f64; 3]) -> ArenaRun {
    ArenaRun {
        commit: ArenaRun::LOCAL_COMMIT.into(),
        champion: None,
        seed: 7,
        league: League::HeadsUp,
        iterations: Some(20),
        rows: ["pricer", "nit", "stablemate loose"]
            .into_iter()
            .zip(values)
            .map(|(name, value)| ArenaRow {
                opponent: name.into(),
                deals: 3_000,
                mbb_per_100: 0.0,
                ci95: 0.0,
                bouts: vec![value; 60],
            })
            .collect(),
    }
}

#[test]
fn a_policy_a_little_behind_the_reference_is_admitted() {
    let (reading, line) = judge(&run([1.; 3]), &run([0.; 3]), MARGIN).unwrap();
    assert_eq!(reading.admission, Admission::Admitted);
    assert!(line.contains("heads-up") && line.ends_with("admitted"));
}

#[test]
fn a_stablemate_row_does_not_enter_the_pool() {
    let (reading, _) = judge(&run([1.; 3]), &run([0., 0., -1e9]), MARGIN).unwrap();
    assert_eq!(reading.admission, Admission::Admitted);
    assert_eq!(reading.mean, -2_000.0);
}

#[test]
fn runs_that_are_not_the_same_bouts_are_refused() {
    let base = run([0.; 3]);
    for (candidate, why) in [
        (
            ArenaRun {
                seed: 9,
                ..base.clone()
            },
            "seed",
        ),
        (
            ArenaRun {
                league: League::SixMax,
                ..base.clone()
            },
            "league",
        ),
        (
            ArenaRun {
                iterations: None,
                ..base.clone()
            },
            "iterations",
        ),
    ] {
        assert!(judge(&base, &candidate, MARGIN).unwrap_err().contains(why));
    }
    let mut candidate = base.clone();
    candidate.rows[0].bouts.pop();
    assert!(judge(&base, &candidate, MARGIN).is_err());
    candidate.rows[0].opponent = "maniac".into();
    assert!(judge(&base, &candidate, MARGIN).is_err());
}

fn fixture(name: &str, store: bool) -> (PathBuf, Fleet) {
    let dir = std::env::temp_dir().join(format!("svanbot-play-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    if store {
        std::fs::write(dir.join("Svanar.db"), "").unwrap();
    }
    let fleet = Fleet::in_dir(dir.join("fleet"));
    (dir, fleet)
}

fn check(admitted: bool) -> Check {
    Check {
        kind: PolicyKind::Solving,
        style: "tight".into(),
        reference: Params::DEFAULT.digest(),
        seed: 7,
        iterations: Some(20),
        bouts: 60,
        admitted,
        heads_up: "heads-up evidence".into(),
        six_max: "6-max evidence".into(),
        checked_ms: 1,
    }
}

fn assign_tight(fleet: &Fleet, dir: &Path) -> Result<String, String> {
    assign(fleet, dir, "Svanar", PolicyKind::Solving, &TIGHT, 2)
}

#[test]
fn a_bot_is_assigned_only_with_a_current_admitted_check() {
    let (dir, fleet) = fixture("check", true);
    assert!(assign_tight(&fleet, &dir).unwrap_err().contains("check"));
    fleet.record_check(&check(false)).unwrap();
    assert!(assign_tight(&fleet, &dir).unwrap_err().contains("admitted"));
    let stale = Check {
        reference: Params { exploit: 0.3 }.digest(),
        ..check(true)
    };
    fleet.record_check(&stale).unwrap();
    assert!(assign_tight(&fleet, &dir).unwrap_err().contains("stale"));
    fleet.record_check(&check(true)).unwrap();
    assign_tight(&fleet, &dir).unwrap();
    assert_eq!(
        fleet.assignment("Svanar").unwrap().unwrap().style.name,
        "tight"
    );
}

#[test]
fn a_bot_with_no_store_is_refused_and_nothing_is_written() {
    let (dir, fleet) = fixture("no-store", false);
    fleet.record_check(&check(true)).unwrap();
    assert!(assign_tight(&fleet, &dir).unwrap_err().contains("store"));
    assert!(!dir.join("fleet/plays.json").exists());
}

#[test]
fn seed_creates_a_slot_from_the_reference_for_a_bot_with_a_store() {
    let (dir, fleet) = fixture("seed-cli", true);
    fleet.promote(&Params { exploit: 0.4 }).unwrap();
    let output = in_fleet(&dir, &["seed", "Svanar"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fleet.champion_slot("Svanar"),
        Ok(Some(Params { exploit: 0.4 }))
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("champion"));
    // A second seed is refused: a learned champion is not a seed's to overwrite.
    let again = in_fleet(&dir, &["seed", "Svanar"]);
    assert_eq!(again.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already"));
}

#[test]
fn seed_refuses_a_bot_with_no_store_and_writes_nothing() {
    let (dir, _) = fixture("seed-no-store", false);
    let output = in_fleet(&dir, &["seed", "Svanar"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("store"));
    assert!(!dir.join("fleet/champions.json").exists());
}

#[test]
fn a_promotion_after_assignment_does_not_unassign() {
    let (dir, fleet) = fixture("promotion", true);
    fleet.record_check(&check(true)).unwrap();
    assign_tight(&fleet, &dir).unwrap();
    fleet.promote(&Params { exploit: 0.3 }).unwrap();
    assert!(fleet.assignment("Svanar").unwrap().is_some());
    assert!(assign_tight(&fleet, &dir).unwrap_err().contains("stale"));
}

#[test]
fn assigning_the_reference_is_a_revert() {
    let (dir, fleet) = fixture("reference", true);
    fleet.record_check(&check(true)).unwrap();
    assign_tight(&fleet, &dir).unwrap();
    assign(&fleet, &dir, "Svanar", PolicyKind::Solving, &STANDARD, 3).unwrap();
    assert!(fleet.assignment("Svanar").unwrap().is_none());
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

#[test]
fn assign_without_a_check_is_refused_with_exit_2() {
    let (dir, _) = fixture("cli-refused", true);
    let output = in_fleet(&dir, &["assign", "Svanar", "solving", "tight"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("check"));
}

#[test]
fn status_lists_assignments_and_checks_and_marks_a_stale_check() {
    let (dir, fleet) = fixture("cli-status", true);
    fleet.record_check(&check(true)).unwrap();
    assign_tight(&fleet, &dir).unwrap();
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

mod guard {
    use super::{Fleet, Params, PolicyKind, TIGHT, fixture};
    use std::path::Path;
    use svanbot::play::guard::{Guarded, guard, judge};
    use svanbot_runtime::{fleet::Assignment, ledger::Played, store::Store};

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
}

mod sequential {
    use super::{ArenaRun, League, run};
    use svanbot::play::check_bar;
    use svanbot_learn::margin::MARGIN;
    use svanbot_runtime::ship::Bar;

    fn comparison(bouts: usize, heads: f64, six: f64) -> [ArenaRun; 4] {
        let mut runs = [run([0.; 3]), run([heads; 3]), run([0.; 3]), run([six; 3])];
        for (index, measured) in runs.iter_mut().enumerate() {
            if index >= 2 {
                measured.league = League::SixMax;
            }
            for row in &mut measured.rows {
                row.bouts.truncate(bouts);
            }
        }
        runs
    }

    #[test]
    fn a_check_reads_nothing_on_fewer_than_thirty_bouts() {
        assert_eq!(check_bar(&comparison(29, -20., -20.), MARGIN), Bar::Open);
    }

    #[test]
    fn a_check_refuses_the_moment_a_league_is_refused() {
        assert!(matches!(
            check_bar(&comparison(60, 0., -20.), MARGIN),
            Bar::Refused(_)
        ));
    }

    #[test]
    fn a_check_clears_only_when_both_leagues_are_admitted() {
        let mut runs = comparison(60, 0., 0.);
        assert_eq!(check_bar(&runs, MARGIN), Bar::Clears);
        for row in &mut runs[3].rows {
            row.bouts = [100., -100.].repeat(30);
        }
        assert_eq!(check_bar(&runs, MARGIN), Bar::Open);
    }
}
