//! The status file: one bot's last known live state, beside its store, for the dashboard to read.
// A test status is a file in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_runtime::status::{Fleet, Seat, Status, Table, status_path};

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-status-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn status(bot: &str, updated_ms: i64) -> Status {
    Status {
        bot: bot.into(),
        pid: 4242,
        version: "11.0.0-dev".into(),
        started_ms: 1_000,
        updated_ms,
        connected: true,
        table: Some(Table {
            id: "t-1".into(),
            seat: Some(2),
            hand_id: Some("h-1".into()),
            street: Some("flop".into()),
            board: vec!["Ah".into(), "Kd".into(), "7c".into()],
            pot: 1_200,
            big_blind: 100,
            dealer: 0,
            players: vec![
                Seat {
                    seat: 0,
                    name: "alpha_bot".into(),
                    stack: 1_800,
                    folded: false,
                },
                Seat {
                    seat: 2,
                    name: bot.into(),
                    stack: 2_000,
                    folded: false,
                },
            ],
        }),
        hands: 42,
        incidents: 0,
        run_incidents: 0,
        last_hand_ms: Some(900),
        last_error: None,
    }
}

#[test]
fn a_status_round_trips_through_its_file() {
    let path = scratch("round-trip").join("SurSvan.status.json");
    let written = status("SurSvan", 2_000);
    written.write(&path).unwrap();
    assert_eq!(Status::read(&path).unwrap(), written);
}

#[test]
fn the_status_file_is_private_to_its_user() {
    use std::os::unix::fs::PermissionsExt;

    let path = scratch("private").join("SurSvan.status.json");
    status("SurSvan", 2_000).write(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn a_second_write_replaces_the_first_and_leaves_no_temporary_behind() {
    let dir = scratch("replace");
    let path = dir.join("SurSvan.status.json");
    status("SurSvan", 2_000).write(&path).unwrap();
    status("SurSvan", 3_000).write(&path).unwrap();
    assert_eq!(Status::read(&path).unwrap().updated_ms, 3_000);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["SurSvan.status.json"]);
}

#[test]
fn a_file_that_is_not_a_status_is_refused_with_a_reason() {
    let path = scratch("garbage").join("SurSvan.status.json");
    std::fs::write(&path, "not json").unwrap();
    let error = Status::read(&path).unwrap_err().to_string();
    assert!(error.contains("status"), "{error}");
}

#[test]
fn the_status_file_sits_beside_the_store() {
    assert_eq!(
        status_path(std::path::Path::new("/data/SurSvan.db")),
        PathBuf::from("/data/SurSvan.status.json")
    );
}

#[test]
fn the_fleet_reads_each_bot_s_status_in_name_order_and_reports_the_rest() {
    let dir = scratch("fleet");
    status("Alpha", 2_000)
        .write(&dir.join("Alpha.status.json"))
        .unwrap();
    status("Beta", 2_100)
        .write(&dir.join("Beta.status.json"))
        .unwrap();
    std::fs::write(dir.join("broken.status.json"), "{").unwrap();
    // A status filed under another bot's name would pool two bots' state: refuse it.
    status("Alpha", 2_200)
        .write(&dir.join("Imposter.status.json"))
        .unwrap();
    std::fs::write(dir.join("notes.txt"), "not a status").unwrap();

    let fleet = Fleet::read(&dir).unwrap();
    let names: Vec<&str> = fleet
        .bots
        .iter()
        .map(|status| status.bot.as_str())
        .collect();
    assert_eq!(names, ["Alpha", "Beta"]);
    assert_eq!(fleet.problems.len(), 2, "{:?}", fleet.problems);
    assert!(fleet.problems.iter().any(|p| p.contains("broken")));
    assert!(fleet.problems.iter().any(|p| p.contains("Imposter")));
}

#[test]
fn a_missing_data_directory_is_an_error_not_an_empty_fleet() {
    let dir = scratch("missing").join("gone");
    assert!(Fleet::read(&dir).is_err());
}
