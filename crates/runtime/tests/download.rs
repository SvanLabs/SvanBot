//! The season download (V11-13) against a stand-in venue: pages into the store as history,
//! duplicates kept once, and a second run resuming where the first stopped.
// The stand-in venue is a socket server, and each store is a file in a temporary directory.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use svanbot_runtime::download::{PAGE, download};
use svanbot_runtime::limit::Limiter;
use svanbot_runtime::record::Hand;
use svanbot_runtime::rest::{EMPTY_PAGE_TRIES, Rest};
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
fn it_pages_the_history_into_the_store_and_resumes_where_it_stopped() {
    let (store, _) = store("resume");
    // Live play already kept one of these hands: the download keeps that first copy.
    let live = Hand {
        hand_id: "h-3".into(),
        table_id: "t-1".into(),
        frames: vec![json!({"type": "hand_start"}).to_string()],
        traced: Vec::new(),
    };
    store.append_hand(&live, Source::Live, 1).unwrap();
    let full = PAGE;
    let pages = vec![
        json!({ "hands": hands(0..full) }),
        json!({ "hands": [{"id": 9999}, {"no_id": true}] }),
    ];
    let (base, served) = venue(pages);
    let summary = download(&mut rest(&base), &store, || 5).unwrap();
    let paths = served.join().unwrap();
    assert_eq!(
        paths[0],
        format!("/api/me/hand-history?limit={full}&offset=0")
    );
    assert_eq!(
        paths[1],
        format!("/api/me/hand-history?limit={full}&offset={full}")
    );
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
    // The second run starts at the offset the first reached, and finds nothing new.
    let (base, served) = venue((0..EMPTY_PAGE_TRIES).map(|_| json!([])).collect());
    let again = download(&mut rest(&base), &store, || 6).unwrap();
    let paths = served.join().unwrap();
    assert!(
        paths[0].ends_with(&format!("offset={}", summary.offset)),
        "{paths:?}"
    );
    assert_eq!((again.pages, again.added), (0, 0));
}
