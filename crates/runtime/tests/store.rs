//! The store: one SQLite file per bot, in WAL mode, whose hands and incidents are append-only.
// A test store is a file in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use serde_json::{Value, json};
use svanbot_runtime::record::{Hand, Recorder};
use svanbot_runtime::session::Incident;
use svanbot_runtime::store::{Source, Store};
use svanbot_runtime::wire::parse_frame;

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-store-{}-{name}", std::process::id()));
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
    }
}

#[test]
fn a_hand_is_kept_with_its_frames_once_and_survives_a_reopen() {
    let path = scratch("reopen").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    assert!(
        store
            .append_hand(&hand("h-1"), Source::Live, 1_000)
            .unwrap()
    );
    // The same hand again, say from the season download, is not a second hand.
    assert!(
        !store
            .append_hand(&hand("h-1"), Source::History, 2_000)
            .unwrap()
    );
    store
        .append_hand(&hand("h-2"), Source::Live, 3_000)
        .unwrap();
    drop(store);
    let store = Store::open(&path, "SurSvan").unwrap();
    let hands = store.hands().unwrap();
    assert_eq!(hands, [hand("h-1"), hand("h-2")]);
    assert_eq!(store.journal_mode().unwrap(), "wal");
}

#[test]
fn a_hand_or_an_incident_cannot_be_changed_or_deleted() {
    let path = scratch("append-only").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    store
        .append_hand(&hand("h-1"), Source::Live, 1_000)
        .unwrap();
    store
        .append_incident(
            &Incident::SharedTable {
                table_id: "t-1".into(),
                stablemate: "Svanar".into(),
            },
            1_000,
        )
        .unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    for sql in [
        "UPDATE hands SET table_id = 't-2'",
        "DELETE FROM hands",
        "UPDATE incidents SET stablemate = 'nobody'",
        "DELETE FROM incidents",
    ] {
        let refused = raw.execute(sql, []).unwrap_err().to_string();
        assert!(refused.contains("append-only"), "{sql}: {refused}");
    }
}

#[test]
fn every_incident_is_kept_in_order() {
    let store = Store::open(&scratch("incidents").join("SurSvan.db"), "SurSvan").unwrap();
    let shared_table = Incident::SharedTable {
        table_id: "t-1".into(),
        stablemate: "Svanar".into(),
    };
    let shared_hand = Incident::SharedHand {
        hand_id: "h-1".into(),
        stablemate: "Svanism".into(),
    };
    store.append_incident(&shared_table, 1_000).unwrap();
    store.append_incident(&shared_hand, 2_000).unwrap();
    store.append_incident(&shared_table, 3_000).unwrap();
    assert_eq!(
        store.incidents().unwrap(),
        [shared_table.clone(), shared_hand, shared_table]
    );
}

#[test]
fn the_recorder_turns_the_frames_of_one_hand_into_one_hand() {
    let mut recorder = Recorder::default();
    let texts = [
        json!({"type": "connected", "agent_id": "a-1", "name": "SurSvan"}),
        json!({"type": "table_joined", "table_id": "t-9", "seat": 2, "players": []}),
        json!({"type": "hand_start", "hand_id": "h-5", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
        json!({"type": "hole_cards", "hand_id": "h-5", "cards": ["Ah", "Kd"]}),
        json!({"type": "player_action", "hand_id": "h-5", "seat": 0, "action": "fold",
               "amount": null, "stack": 1990}),
        json!({"type": "hand_result", "hand_id": "h-5", "final_stacks": {"2": 2010}}),
    ]
    .map(|frame| frame.to_string());
    let mut finished = Vec::new();
    for text in &texts {
        let frame = parse_frame(text).unwrap();
        finished.extend(recorder.record(text, &frame));
    }
    let [hand] = &finished[..] else {
        panic!("{finished:?}")
    };
    assert_eq!(
        (hand.hand_id.as_str(), hand.table_id.as_str()),
        ("h-5", "t-9")
    );
    let kinds: Vec<Value> = hand
        .frames
        .iter()
        .map(|text| serde_json::from_str::<Value>(text).unwrap()["type"].clone())
        .collect();
    assert_eq!(
        kinds,
        ["hand_start", "hole_cards", "player_action", "hand_result"]
    );
}

#[test]
fn a_hand_that_ended_while_away_is_closed_by_the_resync_that_replays_its_result() {
    let mut recorder = Recorder::default();
    for frame in [
        json!({"type": "table_joined", "table_id": "t-9", "seat": 2, "players": []}),
        json!({"type": "hand_start", "hand_id": "h-5", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
    ] {
        let text = frame.to_string();
        assert_eq!(recorder.record(&text, &parse_frame(&text).unwrap()), None);
    }
    let resync = json!({"type": "resync_response", "role": "player", "replayed_events": [
        {"type": "hand_result", "table_seq": 9, "hand_id": "h-5", "final_stacks": {"2": 2010}}
    ]})
    .to_string();
    let hand = recorder.record(&resync, &parse_frame(&resync).unwrap());
    assert_eq!(hand.map(|hand| hand.frames.len()), Some(2));
}

#[test]
fn a_store_belongs_to_one_bot_and_refuses_any_other() {
    let path = scratch("one-bot").join("SurSvan.db");
    drop(Store::open(&path, "SurSvan").unwrap());
    // Two units pointed at one file would pool the bots' reads. The second is refused.
    let refused = Store::open(&path, "Svanar").unwrap_err().to_string();
    assert!(refused.contains("SurSvan"), "{refused}");
    assert!(Store::open(&path, "SurSvan").is_ok());
    let raw = rusqlite::Connection::open(&path).unwrap();
    let changed = raw.execute("UPDATE meta SET value = 'Svanar' WHERE key = 'bot'", []);
    assert!(changed.unwrap_err().to_string().contains("append-only"));
}
