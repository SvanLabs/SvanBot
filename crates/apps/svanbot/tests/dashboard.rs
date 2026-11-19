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
use svanbot_strategy::params::Params;

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
    let fleet_dir = fleet::beside_store(data.join("Alpha.db"));
    let arena = Fleet::in_dir(&fleet_dir);
    // The run is of the champion in play: the default until a promotion. A scoreboard of any
    // other champion is a problem the page raises, which most tests here are not about.
    let champion = arena.champion().unwrap().digest().to_string();
    arena
        .record_arena(&ArenaRun {
            commit: "abc1234".into(),
            champion,
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
        fleet_dir,
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

/// A status with the bot's store beside it, holding one traced hand and one untraced one.
fn traced_store(data: &std::path::Path, bot: &str) {
    use svanbot_core::rules::Street;
    use svanbot_runtime::record::Hand;
    use svanbot_runtime::store::{Source, Store};
    use svanbot_strategy::policy::{Branch, PricedTrace, Ranged, Trace};

    let path = data.join(format!("{bot}.db"));
    let store = Store::open(&path, bot).unwrap();
    for (id, ms, traced) in [("h-1", 1_000, true), ("h-2", 2_000, false)] {
        store
            .append_hand(
                &Hand {
                    hand_id: id.into(),
                    table_id: "t-1".into(),
                    frames: vec![
                        format!(r#"{{"type":"hand_start","hand_id":"{id}"}}"#),
                        format!(r#"{{"type":"hand_result","hand_id":"{id}"}}"#),
                    ],
                    traced: if traced {
                        vec![Trace {
                            branch: Branch::Priced,
                            street: Street::Preflop,
                            solved: None,
                            priced: Some(PricedTrace {
                                equity: 0.61,
                                price: Some(0.25),
                                ranges: vec![Ranged {
                                    seat: 0,
                                    masses: vec![0.0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.1, 0.0],
                                }],
                            }),
                        }]
                    } else {
                        Vec::new()
                    },
                },
                Source::Live,
                ms,
            )
            .unwrap();
    }
}

#[test]
fn the_hands_route_lists_a_bots_traced_hands_newest_first() {
    let (addr, dir) = serve("hands-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/hands/Alpha");
    assert_eq!(code, 200);
    let listed: Value = serde_json::from_str(&body).unwrap();
    let rows = listed["hands"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only the traced hand is listed");
    assert_eq!(rows[0]["hand_id"], "h-1");
    assert_eq!(rows[0]["decisions"], 1);
}

#[test]
fn the_trace_route_reads_one_hands_decisions() {
    let (addr, dir) = serve("trace-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/trace/Alpha/h-1");
    assert_eq!(code, 200);
    let trace: Value = serde_json::from_str(&body).unwrap();
    let decisions = trace["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["branch"], "priced");
    assert_eq!(decisions[0]["street"], "preflop");
    assert_eq!(decisions[0]["price"], 0.25);
}

#[test]
fn an_untraced_hand_says_so_rather_than_rendering_an_empty_trace() {
    let (addr, dir) = serve("untraced-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/trace/Alpha/h-2");
    assert_eq!(code, 200);
    let trace: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(trace["traced"], false);
    assert!(trace["decisions"].as_array().unwrap().is_empty());
    assert!(
        trace["note"].as_str().unwrap().contains("no trace"),
        "the page is told why it has nothing to show"
    );
}

#[test]
fn a_hand_the_store_does_not_hold_is_a_404() {
    let (addr, dir) = serve("missing-hand-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, _) = get(addr, "/api/trace/Alpha/h-missing");
    assert_eq!(code, 404);
}

#[test]
fn a_bot_with_no_store_lists_no_hands_rather_than_failing() {
    // A bot that has just started has decided nothing yet, and the page asks before its store
    // exists.
    let (addr, _dir) = serve("no-store-route");
    let (code, body) = get(addr, "/api/hands/Nobody");
    assert_eq!(code, 200);
    let listed: Value = serde_json::from_str(&body).unwrap();
    assert!(listed["hands"].as_array().unwrap().is_empty());
}

#[test]
fn a_hands_page_is_the_dashboard_document_and_nothing_else_is() {
    // The page is one document the script routes: `/bot/Alpha/hand/h-1` is the replayer's page,
    // and anything deeper is not a page at all.
    let (addr, _dir) = serve("hand-page");
    let (code, body) = get(addr, "/bot/Alpha/hand/h-1");
    assert_eq!(code, 200);
    assert!(body.contains("<!doctype html"), "the dashboard document");
    let (code, _) = get(addr, "/bot/Alpha/hand/h-1/extra");
    assert_eq!(code, 404);
    let (code, _) = get(addr, "/bot/Alpha/hand/../../etc/passwd");
    assert_eq!(code, 404);
}
