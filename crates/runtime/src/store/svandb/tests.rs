//! SvanDB's own tests (V11-128): a reopen, a torn tail, a bad checksum, the trace cap, and the
//! migration gate's bit-exact read comparison against SQLite.

use std::path::PathBuf;

use serde_json::json;
use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_learn::probe::Arm;
use svanbot_learn::reference::{Side, Standing};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{Abstract, Branch, PricedTrace, Ranged, ResolvedTrace, Trace};

use super::super::trace::TRACED_HANDS;
use super::super::{Source, Store, StoreError};
use super::Svandb;
use super::index::{HandEntry, Index, TraceRow, visible_trace_rows};
use crate::ledger::Played;
use crate::record::Hand;
use crate::session::Incident;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svandb-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hand(id: &str) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hand_start", "hand_id": id}).to_string(),
            json!({"type": "hand_result", "hand_id": id}).to_string(),
        ],
        traced: Vec::new(),
    }
}

fn export_frame(id: &str) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({
                "hand_id": id,
                "started_at": "2026-11-23T10:00:00Z",
                "ended_at": "2026-11-23T10:01:00Z",
                "profit": 40,
                "big_blind": 20,
                "all_actions": [{}, {}]
            })
            .to_string(),
        ],
        traced: Vec::new(),
    }
}

fn priced_trace() -> Trace {
    Trace {
        branch: Branch::Priced,
        street: Street::Preflop,
        hole: "AhAs".parse().expect("a hole"),
        board: CardSet::EMPTY,
        resolved: None,
        priced: Some(PricedTrace {
            equity: 0.61,
            price: Some(0.25),
            exploit: Some(0.4),
            ranges: vec![Ranged {
                seat: Some(2),
                masses: vec![0.0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.05, 0.05],
                class: None,
                trust: None,
            }],
        }),
    }
}

fn resolved_trace() -> Trace {
    Trace {
        branch: Branch::Resolved,
        street: Street::Flop,
        hole: "AhKh".parse().expect("a hole"),
        board: "QhJhTh".parse().expect("a board"),
        resolved: Some(ResolvedTrace {
            values: vec![1.5, -0.25],
            strategy: vec![0.75, 0.25],
            taken: 0,
            actions: vec![Abstract::Check, Abstract::Bet(200)],
            ranges: vec![Ranged {
                seat: Some(2),
                masses: vec![0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                class: None,
                trust: None,
            }],
        }),
        priced: None,
    }
}

fn traced_hand(id: &str) -> Hand {
    Hand {
        traced: vec![priced_trace(), resolved_trace()],
        ..hand(id)
    }
}

fn played(
    id: &str,
    net: i64,
    orbit: Option<(u64, u64, Arm)>,
    reference: Option<Standing>,
) -> Played {
    Played {
        hand_id: id.into(),
        table_id: "t-1".into(),
        digest: Params::DEFAULT.digest(),
        net,
        luck: 10.0,
        big_blind: 20,
        orbit,
        reference,
    }
}

fn write_store(db: &Store) {
    assert!(
        db.append_hand(&traced_hand("h-1"), Source::Live, 1_000)
            .unwrap()
    );
    // A differing downloaded copy of a kept hand is kept beside it, not as a second hand.
    assert!(
        !db.append_hand(&export_frame("h-1"), Source::History, 1_500)
            .unwrap()
    );
    assert!(db.append_hand(&hand("h-2"), Source::Live, 2_000).unwrap());
    assert!(
        db.append_hand(&export_frame("h-3"), Source::History, 2_500)
            .unwrap()
    );
    let standing = Standing {
        orbit: 7,
        side: Side::Policy,
        chance: 0.9,
        reference: Params::DEFAULT.digest(),
    };
    assert!(
        db.append_played(
            &played("h-1", 50, Some((u64::MAX - 5, 3, Arm::Challenger)), None),
            1_000
        )
        .unwrap()
    );
    assert!(
        db.append_played(&played("h-2", -30, None, Some(standing)), 2_000)
            .unwrap()
    );
    db.append_incident(
        &Incident::SharedTable {
            table_id: "t-1".into(),
            stablemate: "Svanar".into(),
        },
        3_000,
    )
    .unwrap();
    db.append_incident(
        &Incident::SharedHand {
            hand_id: "h-2".into(),
            stablemate: "Svanar".into(),
        },
        3_100,
    )
    .unwrap();
    db.append_probe(11, "started", r#"{"n":1}"#, 4_000).unwrap();
    db.append_probe(11, "harmful", "{}", 4_100).unwrap();
    db.append_probe(12, "started", r#"{"n":2}"#, 4_200).unwrap();
    db.record_download(4_096, 5_000).unwrap();
    db.mark_release("v11.0", 6_000).unwrap();
    db.mark_release("v11.0", 6_500).unwrap();
    db.record_verified_season("season-1", 100, 7_000).unwrap();
    db.record_verified_season("season-1", 200, 7_100).unwrap();
    db.record_verified_season("season-1", 100, 7_200).unwrap();
    db.append_history(&[(hand("h-4"), 8_000)]).unwrap();
}

#[test]
fn a_reopened_log_answers_every_read_the_same() {
    let dir = scratch("reopen");
    let path = dir.join("A.svandb");
    let db = Svandb::open(&path, "A").unwrap();
    assert!(
        db.append_hand(&traced_hand("h-1"), Source::Live, 1_000)
            .unwrap()
    );
    assert!(
        !db.append_hand(&export_frame("h-1"), Source::History, 1_500)
            .unwrap()
    );
    assert!(db.append_hand(&hand("h-2"), Source::Live, 2_000).unwrap());
    assert!(
        db.append_hand(&export_frame("h-3"), Source::History, 2_500)
            .unwrap()
    );
    assert!(
        db.append_played(
            &played("h-1", 50, Some((u64::MAX - 5, 3, Arm::Challenger)), None),
            1_000
        )
        .unwrap()
    );
    db.append_incident(
        &Incident::SharedTable {
            table_id: "t-1".into(),
            stablemate: "Svanar".into(),
        },
        3_000,
    )
    .unwrap();
    db.append_probe(11, "started", r#"{"n":1}"#, 4_000).unwrap();
    db.mark_release("v11.0", 6_000).unwrap();
    db.mark_release("v11.0", 6_500).unwrap();
    let before = (
        db.hands().unwrap(),
        db.played().unwrap(),
        db.incidents().unwrap(),
        db.open_probe().unwrap(),
        db.totals().unwrap(),
        db.traces("h-1").unwrap(),
        db.hand_quality("h-1").unwrap(),
        db.released("v11.0").unwrap(),
    );
    drop(db);
    let reopened = Svandb::open(&path, "A").unwrap();
    assert_eq!(
        (
            reopened.hands().unwrap(),
            reopened.played().unwrap(),
            reopened.incidents().unwrap(),
            reopened.open_probe().unwrap(),
            reopened.totals().unwrap(),
            reopened.traces("h-1").unwrap(),
            reopened.hand_quality("h-1").unwrap(),
            reopened.released("v11.0").unwrap(),
        ),
        before
    );
    assert_eq!(
        reopened.released("v11.0").unwrap(),
        Some(6_000),
        "the first mark stands"
    );
    assert_eq!(reopened.journal_mode().unwrap(), "append-only");
}

#[test]
fn a_torn_tail_is_not_committed_and_a_flipped_byte_is_refused() {
    let dir = scratch("torn");
    let path = dir.join("A.svandb");
    let db = Svandb::open(&path, "A").unwrap();
    db.append_hand(&hand("h-1"), Source::Live, 1_000).unwrap();
    db.append_hand(&hand("h-2"), Source::Live, 2_000).unwrap();
    drop(db);
    let committed = std::fs::metadata(&path).unwrap().len();
    // A kill between the log write and the manifest rewrite: a line past the committed prefix.
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"kind\":\"hand\",\"hand_id\":\"torn\"")
            .unwrap();
    }
    let reopened = Svandb::open(&path, "A").unwrap();
    assert_eq!(
        reopened.totals().unwrap().hands,
        2,
        "the torn line was never committed"
    );
    assert!(!reopened.holds("torn").unwrap());
    drop(reopened);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        committed,
        "the reopen truncated the tail"
    );
    // Flip one byte inside the committed prefix: the manifest's digest refuses the log.
    let mut bytes = std::fs::read(&path).unwrap();
    let at = bytes.iter().position(|byte| *byte == b'{').unwrap() + 2;
    bytes[at] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    let refused = Svandb::open(&path, "A").unwrap_err().to_string();
    assert!(refused.contains("digest"), "{refused}");
}

#[test]
fn a_store_belongs_to_one_bot_and_refuses_any_other() {
    let dir = scratch("owner");
    let path = dir.join("SurSvan.svandb");
    Svandb::open(&path, "SurSvan").unwrap();
    let refused = Svandb::open(&path, "Svanar").unwrap_err().to_string();
    assert!(refused.contains("SurSvan"), "{refused}");
    assert!(Svandb::open(&path, "SurSvan").is_ok());
    assert_eq!(
        Svandb::open_reader(&path).unwrap().bot().unwrap(),
        "SurSvan"
    );
}

#[test]
fn the_trace_cap_keeps_the_newest_hands() {
    let mut index = Index::default();
    for at in 0..=TRACED_HANDS {
        let hand_id = format!("h-{at:04}");
        index.by_id.insert(hand_id.clone(), at);
        index.hands.push(HandEntry {
            hand_id,
            table_id: "t-1".into(),
            source: Source::Live,
            // Oldest first, so the newest trace is the last hand appended.
            recorded_ms: i64::try_from(at).unwrap(),
            frames: Vec::new(),
            quality: None,
            export: None,
            traces: vec![TraceRow {
                street: "preflop".into(),
                branch: "priced".into(),
                json: "{}".into(),
            }],
        });
    }
    let visible = visible_trace_rows(&index);
    assert_eq!(visible.len(), TRACED_HANDS);
    assert_eq!(visible[0], TRACED_HANDS, "the newest hand is first");
    assert!(
        !visible.contains(&0),
        "the oldest trace has fallen off the cap"
    );
}

#[test]
fn migration_reads_the_same_as_sqlite() {
    let dir = scratch("migration");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    write_store(&store);
    let db = Svandb::import(&store, &dir.join("A.svandb")).unwrap();

    assert_eq!(db.bot().unwrap(), store.bot().unwrap());
    assert_eq!(db.totals().unwrap(), store.totals().unwrap());
    assert_eq!(
        db.download_offset().unwrap(),
        store.download_offset().unwrap()
    );
    assert_eq!(db.incidents().unwrap(), store.incidents().unwrap());
    assert_eq!(db.open_probe().unwrap(), store.open_probe().unwrap());
    assert_eq!(db.hands().unwrap(), store.hands().unwrap());
    for id in ["h-1", "h-2", "h-3", "h-4", "missing"] {
        assert_eq!(db.holds(id).unwrap(), store.holds(id).unwrap(), "{id}");
        assert_eq!(
            db.hand_quality(id).unwrap(),
            store.hand_quality(id).unwrap(),
            "{id}"
        );
        assert_eq!(db.traces(id).unwrap(), store.traces(id).unwrap(), "{id}");
    }
    let (mine, theirs) = (db.hands_for_fit().unwrap(), store.hands_for_fit().unwrap());
    assert_eq!(mine.len(), theirs.len());
    for (mine, theirs) in mine.iter().zip(&theirs) {
        assert_eq!(mine.kept, theirs.kept);
        assert_eq!(mine.export, theirs.export);
        assert_eq!(mine.prefer_live, theirs.prefer_live);
        assert_eq!(mine.kept_is_export, theirs.kept_is_export);
    }
    assert_eq!(db.played().unwrap(), store.played().unwrap());
    assert_eq!(
        db.orbits(u64::MAX - 5).unwrap(),
        store.orbits(u64::MAX - 5).unwrap()
    );
    assert_eq!(
        db.last_reference_orbit().unwrap(),
        store.last_reference_orbit().unwrap()
    );
    let reference = Params::DEFAULT.digest();
    assert_eq!(
        db.reference_orbits(&reference, 0).unwrap(),
        store.reference_orbits(&reference, 0).unwrap()
    );
    assert_eq!(
        db.released("v11.0").unwrap(),
        store.released("v11.0").unwrap()
    );
    assert_eq!(
        db.released("none").unwrap(),
        store.released("none").unwrap()
    );
    assert_eq!(
        db.results_since(0).unwrap(),
        store.results_since(0).unwrap()
    );
    assert_eq!(
        db.corrected_since(0).unwrap(),
        store.corrected_since(0).unwrap()
    );
    assert_eq!(db.export_lines().unwrap(), store.export_lines().unwrap());
    assert_eq!(
        db.verified_season("season-1").unwrap(),
        store.verified_season("season-1").unwrap()
    );
    assert_eq!(db.traced_hands().unwrap(), store.traced_hands().unwrap());
    assert_eq!(db.traced_hands().unwrap().len(), 1, "only h-1 was traced");

    // An import writes a new log only.
    assert!(matches!(
        Svandb::import(&store, &dir.join("A.svandb")),
        Err(StoreError(_))
    ));
}
