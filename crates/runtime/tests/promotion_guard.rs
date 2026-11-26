//! Per-bot promotion guards (V11-77): the named bot's own standing reference arm is the live
//! control, the slot changes only on a clear, and an incomplete or harmful guard restores the
//! incumbent.
// A test fleet and stores are fresh files under the build tree.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::paired::Verdict;
use svanbot_learn::promotion::{DESIGNATED, Evidence, GauntletRow};
use svanbot_runtime::fleet::{Assignment, Fleet, Promotion, Status};
use svanbot_runtime::ledger::ORBIT;
use svanbot_runtime::probing::Probing;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::Store;
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::{LOOSE, TIGHT};

const INCUMBENT: Params = Params { exploit: 0.4 };
const CHALLENGER: Params = Params { exploit: 0.32 };
const B_SLOT: Params = Params { exploit: 0.6 };
const B_CHALLENGER: Params = Params { exploit: 0.48 };

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

fn admitted() -> Evidence {
    Evidence {
        incumbent: INCUMBENT,
        challenger: CHALLENGER,
        kind: PolicyKind::Solving,
        style: TIGHT.name,
        fresh: verdict(300.0, 100.0),
        gauntlet: DESIGNATED
            .into_iter()
            .map(|opponent| GauntletRow {
                opponent: opponent.into(),
                verdict: verdict(100.0, 100.0),
            })
            .collect(),
    }
}

fn proposal(guard_hands: u64) -> Promotion {
    Promotion::new("A", admitted(), guard_hands, 20.0)
}

fn b_proposal(guard_hands: u64) -> Promotion {
    let mut evidence = admitted();
    evidence.incumbent = B_SLOT;
    evidence.challenger = B_CHALLENGER;
    evidence.style = LOOSE.name;
    Promotion::new("B", evidence, guard_hands, 20.0)
}

fn fixture(name: &str) -> (PathBuf, Fleet) {
    let dir = scratch(name);
    let fleet = Fleet::in_dir(&dir);
    fleet.freeze(1_000).unwrap();
    fleet.set_champion("A", &INCUMBENT).unwrap();
    fleet.assign("A", tight(), 1).unwrap();
    fleet.set_champion("B", &B_SLOT).unwrap();
    fleet.assign("B", loose(), 1).unwrap();
    (dir, fleet)
}

fn hand(number: u32, stack: i64) -> Hand {
    Hand {
        hand_id: format!("h-{number}"),
        table_id: "t1".to_owned(),
        frames: vec![
            format!(
                r#"{{"type":"hand_start","hand_id":"h-{number}","seat":2,"dealer_seat":0,"blinds":{{"small_blind":10,"big_blind":20}}}}"#
            ),
            format!(
                r#"{{"type":"hand_result","winners":[],"final_stacks":{{"0":1960,"2":{stack}}}}}"#
            ),
        ],
        traced: Vec::new(),
    }
}

/// Plays `hands` hands from `now`, moving the stack by the arm's chips a hand. The first hand
/// establishes the stack and is not counted, as the ledger counts it.
fn play(
    probing: &mut Probing<'_>,
    number: &mut u32,
    stack: &mut i64,
    hands: u32,
    policy: i64,
    reference: i64,
    now: i64,
) {
    for _ in 0..hands {
        if *number > 0 {
            *stack += if probing.on_reference() {
                reference
            } else {
                policy
            };
        }
        probing.on_hand(&hand(*number, *stack), now + i64::from(*number));
        *number += 1;
    }
}

#[test]
fn a_pending_proposal_does_not_start_while_the_live_path_is_disabled() {
    let (dir, fleet) = fixture("disabled");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    fleet.propose_bot(&proposal(18)).unwrap();
    let (mut probing, notes) = Probing::open(&store, fleet.clone(), false, 1_000);
    assert!(
        notes.iter().any(|note| note.contains("disabled")),
        "{notes:?}"
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(INCUMBENT)));
    let (mut number, mut stack) = (0, 1_000_000);
    play(
        &mut probing,
        &mut number,
        &mut stack,
        1 + 2 * ORBIT,
        0,
        0,
        1_001,
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(INCUMBENT)));
    assert_eq!(
        fleet.bot_promotion("A").unwrap().unwrap().status,
        Status::Pending
    );
}

#[test]
fn the_live_guard_starts_on_the_named_bot_only() {
    let (dir, fleet) = fixture("start");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    fleet.propose_bot(&proposal(18)).unwrap();
    let (_probing, notes) = Probing::open_with_live(&store, fleet.clone(), false, 1_000, true);
    assert!(
        notes.iter().any(|note| note.contains("guard started")),
        "{notes:?}"
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(CHALLENGER)));
    assert_eq!(
        fleet.bot_promotion("A").unwrap().unwrap().status,
        Status::Guarding
    );
    assert_eq!(fleet.champion_slot("B"), Ok(Some(B_SLOT)));
    assert_eq!(fleet.bot_promotion("B"), Ok(None));
}

#[test]
fn a_cleared_guard_keeps_the_challenger_in_only_its_own_slot() {
    let (dir, fleet) = fixture("cleared");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    fleet.propose_bot(&proposal(18)).unwrap();
    let (mut probing, _) = Probing::open_with_live(&store, fleet.clone(), false, 1_000, true);
    let (mut number, mut stack) = (0, 1_000_000);
    play(&mut probing, &mut number, &mut stack, 1 + 18, 0, 0, 1_001);
    let promotion = fleet.bot_promotion("A").unwrap().unwrap();
    assert_eq!(promotion.status, Status::Cleared);
    assert_eq!(fleet.champion_slot("A"), Ok(Some(CHALLENGER)));
    assert_eq!(probing.champion(), CHALLENGER);
    assert_eq!(fleet.champion_slot("B"), Ok(Some(B_SLOT)));
    assert_eq!(fleet.bot_promotion("B"), Ok(None));
}

#[test]
fn a_harmful_guard_restores_the_incumbent() {
    let (dir, fleet) = fixture("harmful");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    fleet.propose_bot(&proposal(180)).unwrap();
    let (mut probing, _) = Probing::open_with_live(&store, fleet.clone(), false, 1_000, true);
    let (mut number, mut stack) = (0, 1_000_000);
    play(
        &mut probing,
        &mut number,
        &mut stack,
        1 + 30 * ORBIT,
        -5_000,
        0,
        1_001,
    );
    let promotion = fleet.bot_promotion("A").unwrap().unwrap();
    assert_eq!(promotion.status, Status::Harmful);
    assert_eq!(fleet.champion_slot("A"), Ok(Some(INCUMBENT)));
    assert_eq!(probing.champion(), INCUMBENT);
    assert_eq!(fleet.champion_slot("B"), Ok(Some(B_SLOT)));
}

#[test]
fn a_guard_cut_off_by_a_restart_restores_the_incumbent_and_keeps_the_evidence() {
    let (dir, fleet) = fixture("restart");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    fleet.propose_bot(&proposal(180)).unwrap();
    let (mut probing, _) = Probing::open_with_live(&store, fleet.clone(), false, 1_000, true);
    let (mut number, mut stack) = (0, 1_000_000);
    play(
        &mut probing,
        &mut number,
        &mut stack,
        1 + ORBIT,
        0,
        0,
        1_001,
    );
    drop(probing);

    let (_probing, notes) = Probing::open_with_live(&store, fleet.clone(), false, 2_000, true);
    assert!(
        notes.iter().any(|note| note.contains("cut off")),
        "{notes:?}"
    );
    let promotion = fleet.bot_promotion("A").unwrap().unwrap();
    assert_eq!(promotion.status, Status::Incomplete);
    assert_eq!(promotion.evidence.challenger, CHALLENGER);
    assert_eq!(fleet.champion_slot("A"), Ok(Some(INCUMBENT)));
}

#[test]
fn two_bots_guard_their_own_slots_without_touching_each_other() {
    let (dir, fleet) = fixture("two-bots");
    let a_store = Store::open(&dir.join("A.db"), "A").unwrap();
    let b_store = Store::open(&dir.join("B.db"), "B").unwrap();
    fleet.propose_bot(&proposal(18)).unwrap();
    fleet.propose_bot(&b_proposal(18)).unwrap();
    let (_a, notes) = Probing::open_with_live(&a_store, fleet.clone(), false, 1_000, true);
    assert!(notes.iter().any(|note| note.contains("guard started")));
    assert_eq!(fleet.champion_slot("A"), Ok(Some(CHALLENGER)));
    assert_eq!(fleet.champion_slot("B"), Ok(Some(B_SLOT)));
    assert_eq!(
        fleet.bot_promotion("B").unwrap().unwrap().status,
        Status::Pending
    );
    let (_b, notes) = Probing::open_with_live(&b_store, fleet.clone(), false, 1_000, true);
    assert!(notes.iter().any(|note| note.contains("guard started")));
    assert_eq!(fleet.champion_slot("B"), Ok(Some(B_CHALLENGER)));
    assert_eq!(fleet.champion_slot("A"), Ok(Some(CHALLENGER)));
}

#[test]
fn a_proposal_whose_slot_or_assignment_changed_is_dropped_before_it_starts() {
    let (dir, fleet) = fixture("stale");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    fleet.propose_bot(&proposal(18)).unwrap();
    // The operator moves A's slot: the proposal's incumbent is no longer there.
    let moved = Params { exploit: 0.55 };
    fleet.set_champion("A", &moved).unwrap();
    let (_probing, notes) = Probing::open_with_live(&store, fleet.clone(), false, 1_000, true);
    assert!(
        notes.iter().any(|note| note.contains("dropped")),
        "{notes:?}"
    );
    assert_eq!(
        fleet.bot_promotion("A").unwrap().unwrap().status,
        Status::Superseded
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(moved)));

    // A new proposal against the moved slot, with its assignment changed, is dropped too.
    let mut evidence = admitted();
    evidence.incumbent = moved;
    fleet
        .propose_bot(&Promotion::new("A", evidence, 18, 20.0))
        .unwrap();
    fleet.assign("A", loose(), 2).unwrap();
    let (_probing, notes) = Probing::open_with_live(&store, fleet.clone(), false, 2_000, true);
    assert!(
        notes.iter().any(|note| note.contains("assignment")),
        "{notes:?}"
    );
    assert_eq!(
        fleet.bot_promotion("A").unwrap().unwrap().status,
        Status::Superseded
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(moved)));
}
