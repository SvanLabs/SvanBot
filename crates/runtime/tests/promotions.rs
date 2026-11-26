//! Per-bot promotion proposals (V11-77): evidence-gated, one waiting per bot, isolated between
//! bots and stale when the incumbent or assignment moves.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::paired::Verdict;
use svanbot_learn::promotion::{DESIGNATED, Evidence, GauntletRow};
use svanbot_runtime::fleet::{Assignment, Candidate, Fleet, Promotion, Status};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::{LOOSE, TIGHT};

const INCUMBENT: Params = Params { exploit: 0.4 };
const CHALLENGER: Params = Params { exploit: 0.32 };

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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

fn verdict(mbb: f64, ci95: f64) -> Verdict {
    Verdict {
        deals: 1_600,
        mbb_per_100: mbb,
        ci95,
    }
}

fn evidence(incumbent: Params, style: &'static str, fresh: Verdict, lower: f64) -> Evidence {
    Evidence {
        incumbent,
        challenger: CHALLENGER,
        kind: PolicyKind::Solving,
        style,
        fresh,
        gauntlet: DESIGNATED
            .into_iter()
            .map(|opponent| GauntletRow {
                opponent: opponent.into(),
                verdict: verdict(lower + 100.0, 100.0),
            })
            .collect(),
    }
}

fn admitted(incumbent: Params, style: &'static str) -> Evidence {
    evidence(incumbent, style, verdict(300.0, 100.0), 0.0)
}

/// A frozen fleet with A (tight) on one slot and B (loose) on another.
fn fixture(name: &str) -> (PathBuf, Fleet) {
    let dir = scratch(name);
    let fleet = Fleet::in_dir(&dir);
    fleet.freeze(1_000).unwrap();
    fleet.set_champion("A", &INCUMBENT).unwrap();
    fleet.assign("A", tight(), 1).unwrap();
    fleet.set_champion("B", &Params { exploit: 0.6 }).unwrap();
    fleet.assign("B", loose(), 1).unwrap();
    (dir, fleet)
}

fn proposal(bot: &str, evidence: Evidence) -> Promotion {
    Promotion::new(bot, evidence, 18, 20.0)
}

#[test]
fn a_valid_proposal_waits_for_its_bot_and_survives_a_restart() {
    let (dir, fleet) = fixture("valid");
    let proposed = proposal("A", admitted(INCUMBENT, TIGHT.name));
    assert_eq!(fleet.propose_bot(&proposed), Ok(true));
    let read = fleet.bot_promotion("A").unwrap().unwrap();
    assert_eq!(read.status, Status::Pending);
    assert_eq!(read.evidence, proposed.evidence);
    assert_eq!(fleet.bot_promotion("B"), Ok(None));
    // A restart is a fresh Fleet over the same directory.
    let restarted = Fleet::in_dir(&dir);
    assert_eq!(
        restarted.bot_promotion("A").unwrap().unwrap().evidence,
        proposed.evidence
    );
    assert_eq!(restarted.start_bot_guard("A", 100), Ok(true));
    assert_eq!(
        restarted.finish_bot_guard("A", Status::Cleared, 200, "cleared"),
        Ok(true)
    );
    let reloaded = Fleet::in_dir(&dir);
    let cleared = reloaded.bot_promotion("A").unwrap().unwrap();
    assert_eq!(cleared.status, Status::Cleared);
    assert_eq!(cleared.started_ms, Some(100));
    assert_eq!(cleared.decided_ms, Some(200));
    assert_eq!(cleared.note.as_deref(), Some("cleared"));
}

#[test]
fn a_second_proposal_is_refused_until_the_first_finishes_and_other_bots_are_separate() {
    let (_dir, fleet) = fixture("competing");
    let a = proposal("A", admitted(INCUMBENT, TIGHT.name));
    let b = proposal("B", admitted(Params { exploit: 0.6 }, LOOSE.name));
    assert_eq!(fleet.propose_bot(&a), Ok(true));
    assert_eq!(
        fleet.propose_bot(&a),
        Ok(false),
        "a pending proposal is not replaced"
    );
    assert_eq!(fleet.propose_bot(&b), Ok(true), "B has its own proposal");
    assert!(fleet.bot_promotion("A").unwrap().is_some());
    assert!(fleet.bot_promotion("B").unwrap().is_some());

    assert_eq!(fleet.start_bot_guard("A", 100), Ok(true));
    assert_eq!(
        fleet.start_bot_guard("A", 101),
        Ok(false),
        "only one guard starts"
    );
    assert_eq!(
        fleet.finish_bot_guard("A", Status::Harmful, 200, "harmful"),
        Ok(true)
    );
    // A terminal promotion no longer owns the slot: A may propose again.
    assert_eq!(fleet.propose_bot(&a), Ok(true));
    assert_eq!(
        fleet.bot_promotion("B").unwrap().unwrap().status,
        Status::Pending,
        "A's guard did not touch B"
    );
}

#[test]
fn a_proposal_naming_another_incumbent_or_policy_is_refused() {
    let (_dir, fleet) = fixture("stale");
    let wrong_incumbent = proposal("A", admitted(Params { exploit: 0.7 }, TIGHT.name));
    assert!(
        fleet
            .propose_bot(&wrong_incumbent)
            .unwrap_err()
            .contains("incumbent")
    );
    let wrong_policy = proposal("A", admitted(INCUMBENT, LOOSE.name));
    assert!(
        fleet
            .propose_bot(&wrong_policy)
            .unwrap_err()
            .contains("assignment")
    );
    assert_eq!(fleet.bot_promotion("A"), Ok(None));
}

#[test]
fn incomplete_or_refused_evidence_writes_nothing() {
    let (_dir, fleet) = fixture("gate");
    let mut missing = admitted(INCUMBENT, TIGHT.name);
    missing.gauntlet.pop();
    assert!(
        fleet
            .propose_bot(&proposal("A", missing))
            .unwrap_err()
            .contains("incomplete")
    );
    let refused = proposal(
        "A",
        evidence(INCUMBENT, TIGHT.name, verdict(100.0, 100.0), 0.0),
    );
    assert!(fleet.propose_bot(&refused).unwrap_err().contains("refused"));
    assert_eq!(fleet.bot_promotion("A"), Ok(None));
}

#[test]
fn a_pending_promotion_is_superseded_without_moving_the_slot() {
    let (_dir, fleet) = fixture("supersede");
    let proposed = proposal("A", admitted(INCUMBENT, TIGHT.name));
    fleet.propose_bot(&proposed).unwrap();
    assert_eq!(fleet.supersede_bot("A", 100, "slot changed"), Ok(true));
    assert_eq!(
        fleet.bot_promotion("A").unwrap().unwrap().status,
        Status::Superseded
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(INCUMBENT)));
    // A terminal proposal is replaced by the next one.
    assert_eq!(fleet.propose_bot(&proposed), Ok(true));
}

#[test]
fn a_search_candidate_becomes_a_proposal_that_names_its_bot_and_incumbent() {
    let candidate = Candidate {
        challenger: CHALLENGER,
        kind: PolicyKind::Solving,
        style: TIGHT.name,
        verdict: verdict(300.0, 100.0),
    };
    let gauntlet = admitted(INCUMBENT, TIGHT.name).gauntlet;
    let promotion = Promotion::from_candidate("A", INCUMBENT, &candidate, gauntlet, 18, 20.0);
    assert_eq!(promotion.bot, "A");
    assert_eq!(promotion.evidence.incumbent, INCUMBENT);
    assert_eq!(promotion.evidence.challenger, CHALLENGER);
    assert_eq!(promotion.evidence.kind, PolicyKind::Solving);
    assert_eq!(promotion.evidence.style, TIGHT.name);
    assert_eq!(promotion.evidence.fresh, candidate.verdict);
}

#[test]
fn a_per_bot_promotion_waits_for_the_frozen_reference() {
    let dir = scratch("unfrozen");
    let fleet = Fleet::in_dir(&dir);
    fleet.set_champion("A", &INCUMBENT).unwrap();
    fleet.assign("A", tight(), 1).unwrap();
    let proposed = proposal("A", admitted(INCUMBENT, TIGHT.name));
    assert!(
        fleet
            .propose_bot(&proposed)
            .unwrap_err()
            .contains("not frozen")
    );
}
