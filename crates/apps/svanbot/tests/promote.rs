//! `svanbot play promote <bot>`: a waiting search candidate, played through the designated
//! gauntlet, put forward for the bot's own live guard (V11-82).

#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use svanbot_learn::paired::Verdict;
use svanbot_runtime::fleet::{Candidate, Fleet, Status};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;

fn in_fleet(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .env("SVANBOT_FLEET", dir.join("fleet"))
        .arg("play")
        .args(args)
        .output()
        .unwrap()
}

/// A fleet frozen at the default reference, with `Ada` slotted and, when `fresh` is given, a
/// candidate waiting under that slot.
fn fixture(name: &str, fresh: Option<Verdict>) -> (PathBuf, Params, Option<Params>) {
    let dir = std::env::temp_dir().join(format!("svanbot-promote-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fleet")).unwrap();
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.freeze(1).unwrap();
    let slot = fleet.seed_champion("Ada").unwrap();
    let challenger = fresh.map(|verdict| {
        // A number the solving/standard reference does not read: the arms stay behaviourally
        // identical, so the gauntlet's paired differences are exactly zero and the gate can
        // admit them on a two-bout test run.
        let challenger = slot.with("station_vpip", 0.86).unwrap();
        assert_ne!(
            challenger, slot,
            "the candidate must differ from its incumbent"
        );
        let candidate = Candidate {
            challenger,
            kind: PolicyKind::Solving,
            style: "standard",
            verdict,
        };
        fleet
            .record_bot_search("Ada", &slot, &[], "fixture", 1, Some(&candidate))
            .unwrap();
        challenger
    });
    (dir, slot, challenger)
}

fn above() -> Verdict {
    Verdict {
        deals: 2_000,
        mbb_per_100: 100.0,
        ci95: 10.0,
    }
}

#[test]
fn a_waiting_candidate_clears_the_gauntlet_and_becomes_a_pending_promotion() {
    let (dir, slot, challenger) = fixture("pass", Some(above()));
    let out = in_fleet(&dir, &["promote", "Ada", "2", "1", "1"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let promotion = fleet
        .bot_promotion("Ada")
        .unwrap()
        .expect("the passing evidence is put forward");
    assert_eq!(promotion.status, Status::Pending);
    assert_eq!(promotion.evidence.incumbent, slot);
    assert_eq!(promotion.evidence.challenger, challenger.unwrap());
    assert_eq!(promotion.evidence.kind, PolicyKind::Solving);
    assert_eq!(promotion.evidence.style, "standard");
    assert_eq!(promotion.evidence.fresh, above());
    let names: Vec<_> = promotion
        .evidence
        .gauntlet
        .iter()
        .map(|row| row.opponent.as_str())
        .collect();
    for opponent in [
        "pricer",
        "v10.0.2",
        "calling station",
        "nit",
        "maniac",
        "switching",
        "learning",
    ] {
        assert!(names.contains(&opponent), "{names:?}");
    }
}

#[test]
fn a_candidate_the_fresh_confirmation_did_not_put_above_zero_is_not_proposed() {
    let (dir, _, _) = fixture(
        "below",
        Some(Verdict {
            deals: 2_000,
            mbb_per_100: -100.0,
            ci95: 10.0,
        }),
    );
    let out = in_fleet(&dir, &["promote", "Ada", "1", "1", "1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("not above the incumbent"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        Fleet::in_dir(dir.join("fleet"))
            .bot_promotion("Ada")
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_bot_with_no_waiting_candidate_is_refused() {
    let (dir, _, _) = fixture("empty", None);
    let out = in_fleet(&dir, &["promote", "Ada", "1", "1", "1"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no candidate"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_bot_with_no_champion_slot_is_refused() {
    let (dir, _, _) = fixture("slotless", None);
    let out = in_fleet(&dir, &["promote", "Nobody", "1", "1", "1"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no champion slot"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
