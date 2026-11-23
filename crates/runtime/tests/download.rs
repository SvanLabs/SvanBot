//! The season download (V11-13, V11-38) against a stand-in venue: the season's export into the
//! store as history, duplicates kept once, and a second run reading the season whole again.
// The stand-in venue is a socket server, and each store is a file in a temporary directory.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use svanbot_runtime::download::{MAX_BODY, PAGE, TIMEOUT, download};
use svanbot_runtime::limit::Limiter;
use svanbot_runtime::record::Hand;
use svanbot_runtime::rest::Rest;
use svanbot_runtime::store::{Source, Store};

fn venue(pages: Vec<Value>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    let served = thread::spawn(move || {
        let mut paths = Vec::new();
        for page in pages {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            paths.push(head.split(' ').nth(1).unwrap().to_owned());
            let body = page.to_string();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
        paths
    });
    (base, served)
}

fn store(name: &str) -> (Store, PathBuf) {
    let dir = std::env::temp_dir().join(format!("svanbot-download-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("SurSvan.db");
    (Store::open(&path, "SurSvan").unwrap(), path)
}

fn rest(base: &str) -> Rest {
    Rest::new(base, "k", Limiter::new(100, Duration::from_secs(60))).with_sleep(|_| {})
}

fn hands(ids: std::ops::Range<u32>) -> Value {
    Value::Array(
        ids.map(|id| json!({"hand_id": format!("h-{id}"), "table_id": "t-1"}))
            .collect(),
    )
}

#[test]
fn it_reads_the_season_export_into_the_store_and_reads_it_whole_again() {
    let (store, _) = store("export");
    // Live play already kept one of these hands: the download keeps that first copy.
    let live = Hand {
        hand_id: "h-3".into(),
        table_id: "t-1".into(),
        frames: vec![json!({"type": "hand_start"}).to_string()],
        traced: Vec::new(),
    };
    store.append_hand(&live, Source::Live, 1).unwrap();
    let full = PAGE;
    let export = "/api/portfolio/bots/a-1/hand-history/export?format=json";
    // The bot's id first, then the export: a list a page, newest first, this season only.
    let (base, served) = venue(vec![
        json!({ "agent_id": "a-1", "name": "SurSvan" }),
        hands(0..full),
        json!([{"id": 9999}, {"no_id": true}]),
    ]);
    let summary = download(&mut rest(&base), &store, || 5).unwrap();
    let paths = served.join().unwrap();
    assert_eq!(paths[0], "/api/me");
    assert_eq!(paths[1], format!("{export}&limit={full}&offset=0"));
    assert_eq!(paths[2], format!("{export}&limit={full}&offset={full}"));
    assert_eq!(summary.pages, 2);
    assert_eq!(
        (summary.added, summary.known, summary.skipped),
        (full, 1, 1)
    );
    assert_eq!(summary.offset, u64::from(full) + 2);
    assert_eq!(store.download_offset().unwrap(), summary.offset);
    let kept = store.hands().unwrap();
    assert_eq!(kept.len() as u32, full + 1);
    assert!(kept.iter().any(|hand| hand.hand_id == "9999"));
    // A second run reads the season from its newest hand again, since every new hand moves the
    // offsets, and keeps nothing twice.
    let (base, served) = venue(vec![json!({ "agent_id": "a-1" }), hands(0..2)]);
    let again = download(&mut rest(&base), &store, || 6).unwrap();
    let paths = served.join().unwrap();
    assert_eq!(paths[1], format!("{export}&limit={full}&offset=0"));
    assert_eq!((again.pages, again.added, again.known), (1, 0, 2));
}

#[test]
fn a_season_larger_than_ten_mebibytes_is_read_whole() {
    // A season's export is 30 MB; the client's usual limit is 10 MiB.
    let pad = "x".repeat(1024);
    let page = Value::Array(
        (0..11_000)
            .map(|id| json!({"hand_id": format!("h-{id}"), "pad": pad}))
            .collect(),
    );
    let (store, _) = store("large");
    let (base, served) = venue(vec![json!({ "agent_id": "a-1" }), page.clone()]);
    assert!(download(&mut rest(&base), &store, || 5).is_err());
    served.join().unwrap();
    let (base, served) = venue(vec![json!({ "agent_id": "a-1" }), page]);
    let mut large = rest(&base).with_large_replies(TIMEOUT, MAX_BODY);
    let summary = download(&mut large, &store, || 6).unwrap();
    served.join().unwrap();
    assert_eq!(summary.added, 11_000);
}

#[test]
fn a_profile_without_an_id_stops_the_download() {
    let (store, _) = store("no-id");
    let (base, served) = venue(vec![json!({ "name": "SurSvan" })]);
    assert!(download(&mut rest(&base), &store, || 5).is_err());
    served.join().unwrap();
    assert!(store.hands().unwrap().is_empty());
}
