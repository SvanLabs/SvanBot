//! The two files a bot and its updater use to tell each other about health.
//!
//! `starts`: the bot appends one line each time it comes up, the time and the build it is, so the
//! updater can ask "has this build started three times in five minutes?" and roll it back when the
//! answer is yes. Append-only, and the writer prunes it, so a fleet that runs for a season does not
//! leave a growing file behind.
//!
//! `update`: the updater's request that the bot stop between hands. Existence is the signal; the
//! bot only reads it, and the updater clears it after the swap, so a bot that restarts into a
//! request that is still standing would stop again at once.
// A test health directory is a set of files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::fs;
use std::path::PathBuf;

use svanbot_runtime::health::{Health, Start, WINDOW_MS};

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-health-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

const MINUTE_MS: i64 = 60_000;

#[test]
fn a_bot_that_has_just_come_up_has_one_start_and_is_not_a_crash_loop() {
    let health = Health::in_dir(scratch("fresh"), "SurSvan");
    health.note_start(1_000, "2026.11.16-1").unwrap();
    assert_eq!(
        health.starts(1_000 + MINUTE_MS, WINDOW_MS).unwrap(),
        vec![Start {
            at_ms: 1_000,
            version: "2026.11.16-1".into()
        }]
    );
    assert!(
        !health
            .in_crash_loop(1_000 + MINUTE_MS, WINDOW_MS, 3)
            .unwrap()
    );
}

#[test]
fn three_starts_inside_the_window_are_a_crash_loop() {
    let health = Health::in_dir(scratch("loop"), "SurSvan");
    for minute in 0..3 {
        health
            .note_start(minute * MINUTE_MS, "2026.11.16-2")
            .unwrap();
    }
    assert!(health.in_crash_loop(2 * MINUTE_MS, WINDOW_MS, 3).unwrap());
    // The same three starts against a limit of four: the count is compared, not assumed.
    assert!(!health.in_crash_loop(2 * MINUTE_MS, WINDOW_MS, 4).unwrap());
}

#[test]
fn starts_older_than_the_window_do_not_count() {
    let health = Health::in_dir(scratch("old"), "SurSvan");
    for minute in 0..2 {
        health
            .note_start(minute * MINUTE_MS, "2026.11.16-1")
            .unwrap();
    }
    // Enough starts to trip the limit, but six minutes before the question is asked.
    health
        .note_start(6 * MINUTE_MS + 100, "2026.11.16-2")
        .unwrap();
    let now = 6 * MINUTE_MS + 100;
    assert_eq!(health.starts(now, WINDOW_MS).unwrap().len(), 1);
    assert!(!health.in_crash_loop(now, WINDOW_MS, 3).unwrap());
}

#[test]
fn a_line_torn_by_a_crash_mid_write_is_skipped_not_fatal() {
    let health = Health::in_dir(scratch("torn"), "SurSvan");
    health.note_start(1_000, "2026.11.16-1").unwrap();
    // A start that died between the time and the version: the reader must still see the good line.
    let path = health.starts_path();
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("1200 2026.11.16-2");
    fs::write(&path, text).unwrap();
    let starts = health.starts(2_000, WINDOW_MS).unwrap();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].at_ms, 1_000);
}

#[test]
fn the_start_log_does_not_grow_without_bound() {
    let health = Health::in_dir(scratch("bound"), "SurSvan");
    for start in 0..200 {
        health.note_start(start * 1_000, "2026.11.16-1").unwrap();
    }
    let lines = fs::read_to_string(health.starts_path())
        .unwrap()
        .lines()
        .count();
    assert!(lines <= 32, "the log kept {lines} lines");
    // The newest ones are still the ones kept: a crash loop is judged on the recent past.
    let starts = health.starts(199_000, WINDOW_MS).unwrap();
    assert_eq!(starts.last().unwrap().at_ms, 199_000);
}

#[test]
fn an_update_request_is_visible_until_the_updater_clears_it() {
    let health = Health::in_dir(scratch("request"), "SurSvan");
    assert!(!health.update_requested());
    health.request_update().unwrap();
    // The bot asks again on every read, so reading must not consume it.
    assert!(health.update_requested());
    assert!(health.update_requested());
    health.clear_update().unwrap();
    assert!(!health.update_requested());
    // Clearing a request that has already gone is not a failure: the bot may have stopped first.
    health.clear_update().unwrap();
}

#[test]
fn a_bot_that_has_stopped_says_so_and_the_updater_can_take_it_back() {
    let health = Health::in_dir(scratch("stopped"), "SurSvan");
    assert!(!health.stopped());
    health.note_stop().unwrap();
    assert!(health.stopped());
    health.clear_stop().unwrap();
    assert!(!health.stopped());
}

#[test]
fn a_health_directory_that_is_not_there_yet_reads_as_healthy() {
    let dir = scratch("missing").join("never-made");
    let health = Health::in_dir(dir, "SurSvan");
    assert!(health.starts(0, WINDOW_MS).unwrap().is_empty());
    assert!(!health.update_requested());
    assert!(!health.stopped());
    assert!(!health.in_crash_loop(0, WINDOW_MS, 3).unwrap());
}

#[test]
fn two_bots_in_one_directory_do_not_read_each_others_health() {
    let dir = scratch("two");
    let svanar = Health::in_dir(dir.clone(), "Svanar");
    let sur = Health::in_dir(dir, "SurSvan");
    svanar.note_start(1_000, "2026.11.16-1").unwrap();
    svanar.request_update().unwrap();
    assert_eq!(sur.starts(1_000, WINDOW_MS).unwrap().len(), 0);
    assert!(!sur.update_requested());
}
