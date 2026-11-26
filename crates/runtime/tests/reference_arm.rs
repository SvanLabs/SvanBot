//! A bot's standing reference arm: one orbit in ten plays the frozen reference, every hand
//! records its arm, and a probe in flight owns its hands alone.
// A test store and fleet are files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::probe::{Kind, Verdict};
use svanbot_learn::reference::{self, Side, Standing};
use svanbot_runtime::fleet::{Fleet, Proposal};
use svanbot_runtime::ledger::{ORBIT, Played};
use svanbot_runtime::probing::Probing;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::Store;
use svanbot_strategy::params::{Digest, Params};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

fn guard(hands: u64) -> Proposal {
    Proposal {
        challenger: Params { exploit: 0.9 },
        kind: Kind::Guard { hands },
        harm_margin: 5.0,
        sim_estimate: None,
        max_orbits: 10_000,
        against: None,
    }
}

/// Plays `hands` hands, moving the stack by `policy` or `reference` chips a hand according to the
/// arm the hand is played by. The first hand at the table establishes the stack and is not
/// counted, as the ledger counts it.
fn play(
    probing: &mut Probing<'_>,
    number: &mut u32,
    stack: &mut i64,
    hands: u32,
    policy: i64,
    reference: i64,
) {
    for _ in 0..hands {
        if *number > 0 {
            *stack += if probing.on_reference() {
                reference
            } else {
                policy
            };
        }
        probing.on_hand(&hand(*number, *stack), 1_000);
        *number += 1;
    }
}

fn standing(orbit: u64, side: Side, chance: f64) -> Standing {
    Standing {
        orbit,
        side,
        chance,
        reference: Params::DEFAULT.digest(),
    }
}

fn played(id: &str, net: i64, reference: Option<Standing>) -> Played {
    Played {
        hand_id: id.into(),
        table_id: "t1".into(),
        digest: Params::DEFAULT.digest(),
        net,
        luck: 0.0,
        big_blind: 20,
        orbit: None,
        reference,
    }
}

#[test]
fn every_hand_records_its_reference_digest_and_arm_chance() {
    let dir = scratch("records");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let reference = Params { exploit: 0.4 };
    let policy = Params { exploit: 0.6 };
    fleet.promote(&reference).unwrap();
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut probing = Probing::open(&store, fleet.clone(), false, 500).0;
    // The slot the bot plays by moves between hands; the reference it measures against does not.
    fleet.promote(&policy).unwrap();

    let (mut number, mut stack) = (0, 1_000_000);
    play(&mut probing, &mut number, &mut stack, 1 + 200 * ORBIT, 0, 0);

    let rows = store.played().unwrap();
    assert_eq!(rows.len(), 200 * ORBIT as usize);
    let (mut policy_hands, mut reference_hands) = (0, 0);
    for row in &rows {
        let arm = row.reference.expect("a standing hand has no arm");
        assert!(row.orbit.is_none(), "a standing hand is not a probe hand");
        assert_eq!(arm.reference, reference.digest());
        match arm.side {
            Side::Policy => {
                policy_hands += 1;
                assert_eq!(arm.chance, 1.0 - reference::SHARE);
                assert_eq!(row.digest, policy.digest());
            }
            Side::Reference => {
                reference_hands += 1;
                assert_eq!(arm.chance, reference::SHARE);
                assert_eq!(row.digest, reference.digest());
            }
        }
    }
    assert!(policy_hands > 0 && reference_hands > 0);
    assert_eq!(probing.standing().unwrap().0, 200);

    let orbits = store.reference_orbits(&reference.digest(), 0).unwrap();
    assert_eq!(orbits.len(), 200);
    for orbit in orbits {
        let expected = reference::assign(reference::bot_seed("SurSvan"), orbit.orbit);
        assert_eq!((orbit.side, orbit.chance), expected);
        assert_eq!(orbit.hands, ORBIT);
    }
}

#[test]
fn a_restart_continues_at_the_next_orbit_and_keeps_what_the_arm_counted() {
    let dir = scratch("restart");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut probing = Probing::open(&store, fleet.clone(), false, 500).0;
    let (mut number, mut stack) = (0, 1_000_000);
    play(&mut probing, &mut number, &mut stack, 1 + 6 * ORBIT, 0, 0);
    let last = store
        .played()
        .unwrap()
        .last()
        .unwrap()
        .reference
        .unwrap()
        .orbit;
    assert_eq!(last, 5);
    assert_eq!(
        probing.standing_guard(0, 18, 20.0).unwrap().0,
        Verdict::Cleared
    );
    drop(probing);

    let (mut probing, _) = Probing::open(&store, fleet.clone(), false, 500);
    let (orbit, side, _) = probing.standing().unwrap();
    assert_eq!(orbit, last + 1);
    assert_eq!(
        side,
        reference::assign(reference::bot_seed("SurSvan"), orbit).0
    );
    // The hands the arm played before the restart still count towards its guard.
    assert_eq!(
        probing.standing_guard(0, 18, 20.0).unwrap().0,
        Verdict::Cleared
    );

    play(&mut probing, &mut number, &mut stack, ORBIT, 0, 0);
    let fresh = store.played().unwrap().last().unwrap().reference.unwrap();
    assert_eq!(fresh.orbit, last + 1);
    assert_eq!(fresh.side, side);
}

#[test]
fn a_probe_in_flight_owns_every_hand_and_the_reference_arm_waits() {
    let dir = scratch("probe");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    let (mut number, mut stack) = (0, 1_000_000);
    play(&mut canary, &mut number, &mut stack, 1 + 3, 0, 0);
    assert_eq!(canary.standing().unwrap().0, 0);

    fleet.propose(&guard(60)).unwrap();
    // This hand is the standing arm's last; the probe starts after it.
    play(&mut canary, &mut number, &mut stack, 1, 0, 0);
    let paused = canary.standing().unwrap();
    assert_eq!(paused.0, 1);
    assert!(!canary.on_reference());

    let before = store.played().unwrap().len();
    play(&mut canary, &mut number, &mut stack, 30, 0, 0);
    assert!(
        !canary.on_reference(),
        "the standing arm played during a probe"
    );
    play(&mut canary, &mut number, &mut stack, 30, 0, 0);
    let rows = store.played().unwrap();
    for row in &rows[before..] {
        assert!(row.reference.is_none(), "a probe hand on the standing arm");
        assert!(row.orbit.is_some(), "a probe hand did not record its probe");
    }
    assert_eq!(canary.standing().unwrap(), paused);

    play(&mut canary, &mut number, &mut stack, ORBIT, 0, 0);
    let fresh = store.played().unwrap().last().unwrap().reference.unwrap();
    assert_eq!(fresh.orbit, paused.0);
    for row in store.played().unwrap() {
        assert!(
            row.orbit.is_none() || row.reference.is_none(),
            "a hand was counted on both the probe and the reference arm"
        );
    }
}

#[test]
fn the_guard_reads_only_the_bot_s_own_reference_hands() {
    let dir = scratch("guard");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let a_store = Store::open(&dir.join("A.db"), "A").unwrap();
    let b_store = Store::open(&dir.join("B.db"), "B").unwrap();
    let mut a = Probing::open(&a_store, fleet.clone(), false, 500).0;
    let mut b = Probing::open(&b_store, fleet.clone(), false, 500).0;
    // A's policy is level with its reference; B's loses two big blinds a hand against its own.
    let (mut number, mut stack) = (0, 1_000_000);
    play(&mut a, &mut number, &mut stack, 1 + 500 * ORBIT, 0, 0);
    let (mut number, mut stack) = (0, 1_000_000);
    play(&mut b, &mut number, &mut stack, 1 + 500 * ORBIT, -40, 0);

    let (a_verdict, _, _) = a.standing_guard(0, 3_000, 20.0).unwrap();
    let (b_verdict, _, _) = b.standing_guard(0, 3_000, 20.0).unwrap();
    assert_eq!(a_verdict, Verdict::Cleared);
    assert_eq!(b_verdict, Verdict::Harmful);
}

#[test]
fn a_control_hand_does_not_count_as_the_policy_s_own_result() {
    let dir = scratch("window");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    store
        .append_played(&played("policy", 100, None), 1_000)
        .unwrap();
    store
        .append_played(
            &played(
                "control",
                -100,
                Some(standing(0, Side::Reference, reference::SHARE)),
            ),
            2_000,
        )
        .unwrap();
    assert_eq!(store.corrected_since(0).unwrap(), [5.0]);
}

#[test]
fn a_store_from_before_the_standing_arm_gains_its_columns_in_place() {
    let dir = scratch("migrate");
    let path = dir.join("A.db");
    let digest = Params::DEFAULT.digest();
    {
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute_batch(&format!(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
             CREATE TABLE played (
                 hand_id     TEXT PRIMARY KEY,
                 table_id    TEXT NOT NULL,
                 recorded_ms INTEGER NOT NULL,
                 digest      TEXT NOT NULL,
                 net         INTEGER NOT NULL,
                 luck        REAL NOT NULL,
                 big_blind   INTEGER NOT NULL CHECK (big_blind > 0),
                 probe       TEXT,
                 orbit       INTEGER,
                 arm         TEXT CHECK (arm IN ('champion', 'challenger'))
             ) STRICT;
             INSERT INTO meta (key, value) VALUES ('bot', 'A');
             INSERT INTO played VALUES ('old', 't1', 1000, '{digest}', 40, 0, 20, NULL, NULL, NULL);"
        ))
        .unwrap();
    }
    let store = Store::open(&path, "A").unwrap();
    assert_eq!(store.played().unwrap()[0].reference, None);
    let arm = Standing {
        orbit: 3,
        side: Side::Policy,
        chance: 1.0 - reference::SHARE,
        reference: digest,
    };
    store
        .append_played(&played("new", 40, Some(arm)), 2_000)
        .unwrap();
    assert_eq!(store.played().unwrap()[1].reference, Some(arm));
}

#[test]
fn a_standing_hand_survives_a_reopen_with_its_reference_digest() {
    let dir = scratch("reopen");
    let path = dir.join("A.db");
    let arm = Standing {
        orbit: 7,
        side: Side::Reference,
        chance: reference::SHARE,
        reference: Params { exploit: 0.4 }.digest(),
    };
    {
        let store = Store::open(&path, "A").unwrap();
        store
            .append_played(&played("h-1", 40, Some(arm)), 1_000)
            .unwrap();
    }
    let store = Store::open(&path, "A").unwrap();
    assert_eq!(store.played().unwrap()[0].reference, Some(arm));
    let digest: Digest = Params { exploit: 0.4 }.digest();
    let orbits = store.reference_orbits(&digest, 0).unwrap();
    assert_eq!(orbits.len(), 1);
    assert_eq!(
        (
            orbits[0].orbit,
            orbits[0].side,
            orbits[0].chance,
            orbits[0].won,
            orbits[0].hands
        ),
        (7, Side::Reference, reference::SHARE, 2.0, 1)
    );
}
