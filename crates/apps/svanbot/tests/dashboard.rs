//! The dashboard server: the fleet API, the event stream and the page, on loopback.
// A test server is a socket and files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use std::io::BufRead;
use std::thread;

use serde_json::Value;
use support::{get, scratch, send, serve, status};
use svanbot::dashboard::{Config, Server};
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet, League};
use svanbot_runtime::status::status_path;
use svanbot_strategy::params::Params;

#[test]
fn the_fleet_endpoint_returns_every_bot_and_the_server_clock() {
    let (addr, _dir) = serve("fleet");
    let (status, body) = get(addr, "/api/fleet");
    assert_eq!(status, 200);
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert!(fleet["server_ms"].as_i64().unwrap() > 0);
    assert_eq!(fleet["bots"][0]["bot"], "Alpha");
    assert_eq!(fleet["bots"][0]["hands"], 12);
    assert_eq!(fleet["problems"], serde_json::json!([]));
}

#[test]
fn the_fleet_says_what_is_installed_and_where_the_slots_point() {
    let (addr, _dir) = serve("release");
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    // By number, not by text; a slot and a staged release are not builds.
    assert_eq!(
        fleet["release"],
        serde_json::json!({"current": "1.10.0", "previous": null, "installed": ["1.9.0", "1.10.0"]})
    );
}

#[test]
fn the_fleet_carries_the_latest_arena_run_and_a_history_to_compare_it_with() {
    let (addr, _dir) = serve("arena");
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    let run = &fleet["arena"][0];
    assert_eq!(run["commit"], "abc1234");
    assert_eq!(run["seed"], 7);
    assert_eq!(run["rows"][0]["opponent"], "nit");
    assert_eq!(run["rows"][0]["deals"], 400);
    assert_eq!(run["rows"][0]["mbb_per_100"], 12_345.6);
    assert_eq!(run["rows"][0]["ci95"], 2_000.4);
}

#[test]
fn the_scoreboard_reads_the_fleet_it_is_given_not_one_beside_the_stores() {
    // `SVANBOT_FLEET` can move the fleet away from `data/fleet` (`crate::fleet_dir`), so the page
    // must read the directory it is told, or it would show an empty scoreboard for a run that
    // just played.
    let dir = scratch("arena-elsewhere");
    let data = dir.join("data");
    let lib = dir.join("lib");
    let elsewhere = dir.join("elsewhere");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    Fleet::in_dir(&elsewhere)
        .record_arena(&ArenaRun {
            commit: "def5678".into(),
            champion: "00000000000000aa".into(),
            seed: 3,
            league: League::HeadsUp,
            rows: vec![ArenaRow {
                opponent: "maniac".into(),
                deals: 100,
                mbb_per_100: 50.0,
                ci95: 10.0,
            }],
        })
        .unwrap();
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data,
        fleet_dir: elsewhere,
        lib_dir: lib,
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fleet["arena"][0]["commit"], "def5678");
}

#[test]
fn with_no_arena_run_the_fleet_carries_an_empty_history() {
    let (addr, dir) = serve("no-arena");
    std::fs::remove_file(dir.join("data/fleet/arena.json")).unwrap();
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fleet["arena"], serde_json::json!([]));
}

#[test]
fn an_arena_run_that_cannot_be_read_is_a_problem_in_the_fleet() {
    // A scoreboard that is silently empty for a run that just played is worse than a page that
    // says the file is broken: the operator has to know the numbers are not there.
    let (addr, dir) = serve("arena-broken");
    std::fs::write(dir.join("data/fleet/arena.json"), "{").unwrap();
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fleet["arena"], serde_json::json!([]));
    assert!(
        fleet["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|problem| problem.as_str().unwrap().contains("arena")),
        "{body}"
    );
}

#[test]
fn a_run_of_a_champion_the_fleet_no_longer_plays_is_a_problem_in_the_fleet() {
    // A promotion changes play without changing the commit, so a scoreboard from before it is
    // not a scoreboard of what is running. The page says so rather than letting the numbers
    // stand for a policy that was replaced.
    let (addr, dir) = serve("arena-promoted");
    // The promotion: a champion the run's line was not played with.
    Fleet::in_dir(dir.join("data/fleet"))
        .promote(&Params {
            exploit: 0.9,
            raise_share: 0.9,
            ..Params::DEFAULT
        })
        .unwrap();
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    let problems: Vec<&str> = fleet["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| problem.as_str().unwrap())
        .collect();
    assert!(
        problems.iter().any(|problem| problem.contains("champion")),
        "{body}"
    );
}

#[test]
fn each_league_is_checked_for_the_champion_in_play() {
    // A 6-max run of the new champion is not a heads-up run of it: the heads-up board still
    // shows the old champion, and the page has to say so though the newest run is current.
    let (addr, dir) = serve("arena-promoted-league");
    let fleet = Fleet::in_dir(dir.join("data/fleet"));
    fleet
        .promote(&Params {
            exploit: 0.9,
            raise_share: 0.9,
            ..Params::DEFAULT
        })
        .unwrap();
    fleet
        .record_arena(&ArenaRun {
            commit: "abc1234".into(),
            champion: fleet.champion().unwrap().digest().to_string(),
            seed: 7,
            league: League::SixMax,
            rows: Vec::new(),
        })
        .unwrap();
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    let problems: Vec<&str> = fleet["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| problem.as_str().unwrap())
        .collect();
    assert_eq!(
        problems
            .iter()
            .filter(|problem| problem.contains("champion"))
            .count(),
        1,
        "{body}"
    );
    assert!(
        problems.iter().any(|problem| problem.contains("heads-up")),
        "{body}"
    );
}

#[test]
fn a_status_that_cannot_be_read_is_a_problem_in_the_fleet() {
    let (addr, dir) = serve("problem");
    std::fs::write(dir.join("data/Broken.status.json"), "{").unwrap();
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert!(
        fleet["problems"][0]
            .as_str()
            .unwrap()
            .contains("Broken.status.json"),
        "{body}"
    );
}

#[test]
fn the_page_its_script_and_its_styles_are_served_and_every_view_is_the_page() {
    let (addr, _dir) = serve("static");
    for path in ["/", "/release", "/scoreboard", "/bot/SurSvan", "/?x=1"] {
        let (status, body) = get(addr, path);
        assert_eq!(status, 200, "{path}");
        assert!(body.contains("<title>SvanBot</title>"), "{path}: {body}");
    }
    let (status, body) = get(addr, "/app.js");
    assert_eq!(status, 200);
    assert!(body.contains("EventSource"), "{body}");
    let (status, body) = get(addr, "/fleet.js");
    assert_eq!(status, 200);
    assert!(body.contains("export function botView"), "{body}");
    // The replayer is its own module, served the same way.
    let (status, body) = get(addr, "/replay.js");
    assert_eq!(status, 200);
    assert!(body.contains("export function handPage"), "{body}");
    let (status, _) = get(addr, "/dom.js");
    assert_eq!(status, 200);
    let (status, body) = get(addr, "/app.css");
    assert_eq!(status, 200);
    assert!(body.contains("prefers-color-scheme: dark"), "{body}");
}

#[test]
fn nothing_but_the_page_is_served() {
    let (addr, _dir) = serve("nothing-else");
    for path in [
        "/missing",
        "/../Cargo.toml",
        "/%2e%2e/Cargo.toml",
        "/bot/../../etc/passwd",
        "/bot/a%2fb",
        "/api/unknown",
    ] {
        assert_eq!(get(addr, path).0, 404, "{path}");
    }
}

#[test]
fn only_a_get_is_answered_and_a_request_that_is_not_one_is_refused() {
    let (addr, _dir) = serve("methods");
    let (status, _) = send(addr, "POST /api/fleet HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(status.contains("405"), "{status}");
    let (status, _) = send(addr, "\r\n\r\n");
    assert!(status.contains("400"), "{status}");
}

#[test]
fn the_dashboard_refuses_a_public_address() {
    let dir = scratch("public");
    let error = Server::bind(Config {
        addr: "0.0.0.0:0".into(),
        data_dir: dir.clone(),
        fleet_dir: dir.join("fleet"),
        lib_dir: dir,
    })
    .err()
    .expect("a public address is refused");
    assert!(error.to_string().contains("loopback"), "{error}");
}

#[test]
fn the_event_stream_sends_the_fleet_at_once_and_again_when_it_changes() {
    let (addr, dir) = serve("events");
    let (head, mut reader) = send(addr, "GET /api/events HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(head.contains("200"), "{head}");

    let mut read_until = |wanted: &str| {
        let mut seen = String::new();
        let mut line = String::new();
        while !seen.contains(wanted) {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0, "the stream ended");
            seen.push_str(&line);
        }
        seen
    };
    let first = read_until("\"Alpha\"");
    assert!(first.contains("event: fleet"), "{first}");

    status("Beta", 2_100)
        .write(&status_path(&dir.join("data/Beta.db")))
        .unwrap();
    let changed = read_until("\"Beta\"");
    assert!(changed.contains("event: fleet"), "{changed}");
}
