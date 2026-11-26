//! A hand's provenance (V11-122): what its frames said about how it was recorded, kept beside
//! the hand — the parser revision, frames that do not parse, whether the hand finished, and the
//! betting amounts the record inferred; plus the duplicate refused rather than counted twice.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use serde_json::{Value, json};
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Store};

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("svanbot-provenance-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hand(id: &str, frames: &[Value]) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: frames.iter().map(|frame| frame.to_string()).collect(),
        traced: vec![],
    }
}

fn quality(store: &Store, id: &str) -> Value {
    let text = store.hand_quality(id).unwrap().expect("a quality mark");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_recorded_hand_carries_its_provenance() {
    let dir = scratch("complete");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    let recorded = hand(
        "h-1",
        &[
            json!({"type": "hand_start", "hand_id": "h-1", "seat": 2, "dealer_seat": 0,
                   "blinds": {"small_blind": 10, "big_blind": 20}}),
            // A call whose amount the venue left out: read as a call, its size inferred.
            json!({"type": "player_action", "hand_id": "h-1", "seat": 0, "action": "call",
                   "amount": null, "stack": 1980}),
            json!({"type": "player_action", "hand_id": "h-1", "seat": 2, "action": "fold",
                   "amount": null, "stack": 1990}),
            json!({"type": "hand_result", "hand_id": "h-1", "final_stacks": {"2": 2010}}),
        ],
    );
    assert!(store.append_hand(&recorded, Source::Live, 1).unwrap());
    let quality = quality(&store, "h-1");
    assert_eq!(quality["parser"], "2026-09-02");
    assert_eq!(quality["frames"], 4);
    assert_eq!(quality["unparsed"], 0);
    assert_eq!(quality["complete"], true);
    // The call's amount was null — inferred; the fold's null is the venue's own spelling.
    assert_eq!(quality["inferred_amounts"], 1);
}

#[test]
fn an_unfinished_hand_with_a_broken_frame_says_so() {
    let dir = scratch("broken");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    let mut frames = vec![
        json!({"type": "hand_start", "hand_id": "h-2", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
    ];
    let mut broken = hand("h-2", &frames);
    // A frame the recorder kept that has no `type` to route it by: the mark counts it, and the
    // hand is not complete — a missing result is not a result of no chips. (A frame that is not
    // even JSON never reaches the store: its `json_valid` check refuses the hand entirely.)
    broken.frames.push(json!({"no_type": true}).to_string());
    frames.clear();
    assert!(store.append_hand(&broken, Source::Live, 2).unwrap());
    let quality = quality(&store, "h-2");
    assert_eq!(quality["unparsed"], 1);
    assert_eq!(quality["complete"], false);
    assert_eq!(quality["inferred_amounts"], 0);
}

#[test]
fn a_duplicate_hand_is_refused_not_counted_twice() {
    let dir = scratch("duplicate");
    let store = Store::open(&dir.join("A.db"), "A").unwrap();
    let recorded = hand(
        "h-3",
        &[json!({"type": "hand_result", "hand_id": "h-3", "final_stacks": {"2": 2010}})],
    );
    assert!(store.append_hand(&recorded, Source::Live, 3).unwrap());
    assert!(!store.append_hand(&recorded, Source::History, 4).unwrap());
    assert_eq!(store.hands().unwrap().len(), 1);
    assert_eq!(quality(&store, "h-3")["complete"], true);
}

#[test]
fn an_old_store_gains_the_column_and_keeps_its_hands() {
    let dir = scratch("old");
    let path = dir.join("A.db");
    // A store recorded before the column existed: the hands table as it was.
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE hands (
                 hand_id     TEXT PRIMARY KEY,
                 table_id    TEXT NOT NULL,
                 source      TEXT NOT NULL CHECK (source IN ('live', 'history')),
                 recorded_ms INTEGER NOT NULL,
                 frames      TEXT NOT NULL CHECK (json_valid(frames))
             ) STRICT;
             INSERT INTO hands VALUES ('h-old', 't-1', 'live', 1,
                 '[{\"type\":\"hand_start\",\"hand_id\":\"h-old\"}]');",
        )
        .unwrap();
    }
    let store = Store::open(&path, "A").unwrap();
    // The old row keeps its hand and reads with unknown provenance: not clean, just unrecorded.
    assert_eq!(store.hand_quality("h-old").unwrap(), None);
    assert_eq!(store.hands().unwrap().len(), 1);
    // A new hand in the migrated store carries its mark.
    let recorded = hand(
        "h-new",
        &[json!({"type": "hand_result", "hand_id": "h-new", "final_stacks": {"2": 2010}})],
    );
    assert!(store.append_hand(&recorded, Source::Live, 2).unwrap());
    assert_eq!(quality(&store, "h-new")["complete"], true);
}
