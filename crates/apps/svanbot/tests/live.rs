//! The bot's status file, kept current while it plays: the dashboard's only live input.
// A test status is a file in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use svanbot::live::Live;
use svanbot_runtime::status::{Play, Seat, Status, Table};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-live-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("SurSvan.status.json")
}

fn status() -> Status {
    Status {
        bot: "SurSvan".into(),
        pid: 1,
        version: "11.0.0-dev".into(),
        started_ms: 1_000,
        updated_ms: 1_000,
        connected: false,
        table: None,
        hands: 7,
        incidents: 1,
        run_incidents: 0,
        last_hand_ms: None,
        last_error: None,
        play: Play::default(),
        opponents: Vec::new(),
        reads_in_play: false,
    }
}

fn table() -> Table {
    Table {
        id: "t-1".into(),
        seat: Some(2),
        hand_id: None,
        street: None,
        board: Vec::new(),
        pot: 0,
        big_blind: 0,
        dealer: 0,
        players: vec![Seat {
            seat: 2,
            name: "SurSvan".into(),
            stack: 2_000,
            folded: false,
        }],
    }
}

#[test]
fn a_started_bot_writes_a_status_before_it_connects() {
    let path = scratch("start");
    let live = Live::start(path.clone(), status()).unwrap();
    let written = Status::read(&path).unwrap();
    assert!(!written.connected);
    assert_eq!(written.hands, 7);
    live.stop();
}

#[test]
fn a_connection_and_a_disconnection_move_the_status() {
    let path = scratch("connection");
    let live = Live::start(path.clone(), status()).unwrap();
    live.connected();
    let written = Status::read(&path).unwrap();
    assert!(written.connected);
    assert!(written.last_error.is_none());

    live.disconnected(Some("the venue refused the key".into()));
    let written = Status::read(&path).unwrap();
    assert!(!written.connected);
    assert_eq!(
        written.last_error.as_deref(),
        Some("the venue refused the key")
    );
    live.stop();
}

#[test]
fn a_disconnection_clears_the_table_it_no_longer_sits_at() {
    let path = scratch("disconnect-table");
    let live = Live::start(path.clone(), status()).unwrap();
    live.table(Some(table()));
    live.disconnected(Some("the socket closed".into()));
    assert_eq!(Status::read(&path).unwrap().table, None);
    live.stop();
}

#[test]
fn a_kept_hand_counts_and_moves_the_last_hand_time() {
    let path = scratch("hand");
    let live = Live::start(path.clone(), status()).unwrap();
    live.hand(true, 5_000);
    let written = Status::read(&path).unwrap();
    assert_eq!((written.hands, written.last_hand_ms), (8, Some(5_000)));
    // A hand the store already held is not a second hand, but it was still seen.
    live.hand(false, 6_000);
    let written = Status::read(&path).unwrap();
    assert_eq!((written.hands, written.last_hand_ms), (8, Some(6_000)));
    live.stop();
}

#[test]
fn an_incident_counts_and_keeps_its_time() {
    let path = scratch("incident");
    let live = Live::start(path.clone(), status()).unwrap();
    live.incident();
    let written = Status::read(&path).unwrap();
    assert_eq!(written.incidents, 2);
    assert_eq!(written.run_incidents, 1);
    live.stop();
}

#[test]
fn the_table_is_written_and_cleared() {
    let path = scratch("table");
    let live = Live::start(path.clone(), status()).unwrap();
    live.table(Some(table()));
    assert_eq!(Status::read(&path).unwrap().table, Some(table()));
    live.table(None);
    assert_eq!(Status::read(&path).unwrap().table, None);
    live.stop();
}

#[test]
fn a_refresh_refreshes_the_document_without_touching_the_state() {
    let path = scratch("beat");
    let live = Live::start(path.clone(), status()).unwrap();
    live.refresh();
    let written = Status::read(&path).unwrap();
    assert!(written.updated_ms >= written.started_ms);
    assert_eq!(written.hands, 7);
    live.stop();
}

#[test]
fn the_refresh_thread_keeps_a_quiet_bot_fresh() {
    let path = scratch("thread");
    let live = Arc::new(Live::start(path.clone(), status()).unwrap());
    let before = Status::read(&path).unwrap().updated_ms;
    let refresh = Live::spawn_refresh(&live, Duration::from_millis(10));
    std::thread::sleep(Duration::from_millis(30));
    live.stop();
    refresh.join().unwrap();
    let after = Status::read(&path).unwrap().updated_ms;
    assert!(after > before, "{before} then {after}");
}

#[test]
fn a_counted_hand_adds_to_the_play_and_the_probe_follows() {
    let path = scratch("played");
    let live = Live::start(path.clone(), status()).unwrap();
    live.played(Some(1_500), Some("a probe".into()), "aa".into());
    live.played(None, None, "aa".into());
    live.played(Some(-500), None, "bb".into());
    let play = Status::read(&path).unwrap().play;
    assert_eq!((play.counted, play.won_mbb), (2, 1_000));
    assert_eq!(play.recent, vec![1_500, -500]);
    assert_eq!((play.probe, play.champion.as_str()), (None, "bb"));
}
