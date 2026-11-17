//! The dashboard server: the fleet API, the event stream and the page, on loopback.
// A test server is a socket and files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use svanbot::dashboard::{Config, Server};
use svanbot_runtime::fleet::{self, ArenaRow, ArenaRun, Fleet};
use svanbot_runtime::status::{Play, Status, status_path};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-dashboard-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn status(bot: &str, updated_ms: i64) -> Status {
    Status {
        bot: bot.into(),
        pid: 1,
        version: "11.0.0-dev".into(),
        started_ms: 1_000,
        updated_ms,
        connected: true,
        table: None,
        hands: 12,
        incidents: 0,
        run_incidents: 0,
        last_hand_ms: Some(900),
        last_error: None,
        play: Play::default(),
    }
}

/// A server on an ephemeral port, with `Alpha` already reporting, two builds installed, and one
/// recorded Arena run in the fleet beside the stores.
fn serve(name: &str) -> (SocketAddr, PathBuf) {
    let dir = scratch(name);
    let data = dir.join("data");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&data).unwrap();
    for version in ["1.9.0", "1.10.0", "staged"] {
        std::fs::create_dir_all(lib.join(version)).unwrap();
    }
    std::os::unix::fs::symlink(lib.join("1.10.0"), lib.join("current")).unwrap();
    status("Alpha", 2_000)
        .write(&status_path(&data.join("Alpha.db")))
        .unwrap();
    Fleet::in_dir(fleet::beside_store(data.join("Alpha.db")))
        .record_arena(&ArenaRun {
            commit: "abc1234".into(),
            seed: 7,
            rows: vec![ArenaRow {
                opponent: "nit".into(),
                deals: 400,
                mbb_per_100: 12_345.6,
                ci95: 2_000.4,
            }],
        })
        .unwrap();
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data,
        lib_dir: lib,
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    (addr, dir)
}

/// Sends `request` as written and returns the status line and a reader at the body.
fn send(addr: SocketAddr, request: &str) -> (String, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    (status, reader)
}

/// A whole answer to `GET path`: the status code and the body.
fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let (status, mut reader) = send(addr, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
    let mut body = String::new();
    reader.read_to_string(&mut body).unwrap();
    (status.split(' ').nth(1).unwrap().parse().unwrap(), body)
}

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
fn with_no_arena_run_the_fleet_carries_an_empty_history() {
    let (addr, dir) = serve("no-arena");
    std::fs::remove_file(dir.join("data/fleet/arena.json")).unwrap();
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fleet["arena"], serde_json::json!([]));
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
