//! The dashboard server: the fleet API, the event stream and the exported page, on loopback.
// A test server is a socket and files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use svanbot::dashboard::{Config, Server};
use svanbot_runtime::status::{Status, status_path};

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
    }
}

/// A server on an ephemeral port, with `Alpha` already reporting and a small export installed.
fn serve(name: &str) -> (SocketAddr, PathBuf) {
    let dir = scratch(name);
    let data = dir.join("data");
    let web = dir.join("web");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(web.join("_next/static")).unwrap();
    std::fs::write(
        web.join("index.html"),
        "<!doctype html><title>SvanBot</title><div id=root></div>",
    )
    .unwrap();
    std::fs::write(web.join("_next/static/app.js"), "console.log('svanbot')").unwrap();
    status("Alpha", 2_000)
        .write(&status_path(&data.join("Alpha.db")))
        .unwrap();
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data,
        web_dir: web,
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    (addr, dir)
}

fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let url = format!("http://{addr}{path}");
    let mut response = agent.get(&url).call().unwrap();
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap();
    (status, body)
}

/// A raw request, for what a URL helper would normalise before it reached the server.
fn raw_get(addr: SocketAddr, path: &str) -> (String, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
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
fn the_exported_page_and_its_assets_are_served() {
    let (addr, _dir) = serve("static");
    let (status, body) = get(addr, "/");
    assert_eq!(status, 200);
    assert!(body.contains("<title>SvanBot</title>"), "{body}");
    let (status, body) = get(addr, "/_next/static/app.js");
    assert_eq!(status, 200);
    assert!(body.contains("svanbot"), "{body}");
    let (status, _) = get(addr, "/missing");
    assert_eq!(status, 404);
}

#[test]
fn a_path_outside_the_export_is_refused() {
    let (addr, _dir) = serve("traversal");
    let (status, _) = raw_get(addr, "/../Cargo.toml");
    assert!(status.contains("404"), "{status}");
    let (status, _) = raw_get(addr, "/%2e%2e/Cargo.toml");
    assert!(status.contains("404"), "{status}");
}

#[test]
fn the_dashboard_refuses_a_public_address() {
    let dir = scratch("public");
    let error = Server::bind(Config {
        addr: "0.0.0.0:0".into(),
        data_dir: dir.clone(),
        web_dir: dir,
    })
    .err()
    .expect("a public address is refused");
    assert!(error.to_string().contains("loopback"), "{error}");
}

#[test]
fn a_missing_export_answers_503_with_a_reason_not_a_404() {
    let dir = scratch("no-export");
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data,
        web_dir: dir.join("gone"),
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    let (status, body) = get(addr, "/");
    assert_eq!(status, 503);
    assert!(body.contains("export"), "{body}");
}

#[test]
fn the_event_stream_sends_the_fleet_at_once_and_again_when_it_changes() {
    let (addr, dir) = serve("events");
    let (head, mut reader) = raw_get(addr, "/api/events");
    assert!(head.contains("200"), "{head}");

    let mut seen = String::new();
    let mut line = String::new();
    while !seen.contains("event: fleet") {
        line.clear();
        reader.read_line(&mut line).unwrap();
        seen.push_str(&line);
    }
    while !seen.contains("\"Alpha\"") {
        line.clear();
        reader.read_line(&mut line).unwrap();
        seen.push_str(&line);
    }

    status("Beta", 2_100)
        .write(&status_path(&dir.join("data/Beta.db")))
        .unwrap();
    let mut changed = String::new();
    while !changed.contains("\"Beta\"") {
        line.clear();
        reader.read_line(&mut line).unwrap();
        changed.push_str(&line);
    }
}
